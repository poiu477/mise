//! Fetch a web page and its photo for an import, from the public internet only.
//!
//! The address comes from whoever is importing, and the server makes the request, so without care it could be
//! pointed at things only the server can reach: its own loopback services, the home network, a cloud host's
//! metadata address. Every request is checked before it is made:
//!
//! - only `http` and `https`, without a username or password, on port 80, 443 or one from 1024 up;
//! - the host is resolved once, and refused when any address it resolves to isn't public ([`is_public`]);
//! - the connection goes to the addresses that were checked, never to a second DNS answer, which could differ;
//! - redirects are followed here, one at a time, each new address checked the same way, at most
//!   [`MAX_REDIRECTS`];
//! - the answer must be the kind asked for (a page, an image), within a size limit, inside a deadline.
//!
//! `IMPORT_ALLOW_PRIVATE=true` lifts the address and port checks, for importing from another app on the home
//! network; the rest stays.

use std::fmt;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use url::{Host, Url};

/// Redirects followed before giving up: a page moved to https, then to www, then to its new address, with room.
pub const MAX_REDIRECTS: usize = 5;
/// The largest page read. Recipe pages run from 100 KB to 2 MB.
pub const MAX_PAGE_BYTES: usize = 5 * 1024 * 1024;
/// The largest photo read: the same as a photo uploaded with a written recipe.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
/// How long connecting to one address may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a page may take from the first request to its last byte, redirects included.
const PAGE_DEADLINE: Duration = Duration::from_secs(20);
/// How long a photo may take, likewise.
const IMAGE_DEADLINE: Duration = Duration::from_secs(30);
/// The longest address fetched, in bytes: redirects can't grow one without end.
const MAX_URL: usize = 2048;

/// Why a fetch didn't happen or didn't give what was asked for. Each reads as the end of "couldn't import that
/// link: …".
#[derive(Debug)]
pub enum FetchError {
    /// The address isn't one mise fetches, whatever is there; the text says why ("only http and https links can
    /// be imported").
    BadAddress(String),
    /// The host is, or resolves to, an address that isn't on the public internet.
    NotPublic {
        /// The host as the address names it.
        host: String,
    },
    /// The host's name has no address.
    NoSuchHost {
        /// The host as the address names it.
        host: String,
    },
    /// Nothing answered at the host, or the connection failed part way.
    Unreachable {
        /// The host as the address names it.
        host: String,
    },
    /// The site took longer than the deadline.
    TimedOut,
    /// The site answered with an error status, such as 404.
    Status(u16),
    /// More than [`MAX_REDIRECTS`] redirects, or one that names no address.
    Redirects,
    /// The answer is larger than the limit, in bytes.
    TooLarge(usize),
    /// The answer isn't what was asked for; `what` is "a web page" or "an image".
    WrongType {
        /// What was asked for.
        what: &'static str,
    },
}

impl FetchError {
    /// Whether mise refused the address itself, before or instead of asking the site: the importer's to fix,
    /// where the others are the site's doing.
    pub fn refused(&self) -> bool {
        matches!(
            self,
            FetchError::BadAddress(_) | FetchError::NotPublic { .. }
        )
    }
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::BadAddress(why) => write!(f, "{why}"),
            FetchError::NotPublic { host } => {
                write!(f, "{host} isn't an address on the public internet")
            }
            FetchError::NoSuchHost { host } => write!(f, "there is no site at {host}"),
            FetchError::Unreachable { host } => write!(f, "{host} didn't answer"),
            FetchError::TimedOut => write!(f, "the site took too long to answer"),
            FetchError::Status(status) => write!(f, "the site answered with an error ({status})"),
            FetchError::Redirects => write!(f, "the site redirected too many times"),
            FetchError::TooLarge(limit) => write!(
                f,
                "what the site sent is larger than {} MB",
                limit / (1024 * 1024)
            ),
            FetchError::WrongType { what } => write!(f, "the link doesn't lead to {what}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Whether an address is on the public internet: not this machine, not a private network, not link-local (where
/// cloud hosts keep their metadata service), and not any range set aside for documentation, tests, multicast or
/// the future. An IPv6 address that carries an IPv4 one is judged by the IPv4 address inside it.
///
/// ```
/// use mise_ingest::fetch::is_public;
///
/// assert!(is_public("93.184.215.14".parse().unwrap()));
/// assert!(!is_public("127.0.0.1".parse().unwrap()));
/// assert!(!is_public("192.168.1.20".parse().unwrap()));
/// assert!(!is_public("169.254.169.254".parse().unwrap()));
/// assert!(!is_public("::ffff:10.0.0.1".parse().unwrap()));
/// ```
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

/// IPv4 ranges that aren't the public internet, as (first address, prefix length), from the IANA special-purpose
/// address registry.
const NOT_PUBLIC_V4: &[([u8; 4], u32)] = &[
    ([0, 0, 0, 0], 8),       // "this network"
    ([10, 0, 0, 0], 8),      // private
    ([100, 64, 0, 0], 10),   // shared between a provider and its customers
    ([127, 0, 0, 0], 8),     // loopback
    ([169, 254, 0, 0], 16),  // link-local, cloud metadata
    ([172, 16, 0, 0], 12),   // private
    ([192, 0, 0, 0], 24),    // protocol assignments
    ([192, 0, 2, 0], 24),    // documentation
    ([192, 88, 99, 0], 24),  // 6to4 relays, retired
    ([192, 168, 0, 0], 16),  // private
    ([198, 18, 0, 0], 15),   // benchmarking
    ([198, 51, 100, 0], 24), // documentation
    ([203, 0, 113, 0], 24),  // documentation
    ([224, 0, 0, 0], 4),     // multicast
    ([240, 0, 0, 0], 4),     // reserved, and broadcast
];

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let bits = u32::from(ip);
    !NOT_PUBLIC_V4.iter().any(|&(first, prefix)| {
        let mask = u32::MAX << (32 - prefix);
        bits & mask == u32::from(Ipv4Addr::from(first)) & mask
    })
}

/// IPv6 ranges inside the global unicast block (`2000::/3`) that aren't the public internet all the same.
const NOT_PUBLIC_V6: &[([u16; 2], u32)] = &[
    ([0x2001, 0x0000], 23), // protocol assignments, Teredo tunnels among them
    ([0x2001, 0x0db8], 32), // documentation
    ([0x2002, 0x0000], 16), // 6to4: an IPv4 address in an IPv6 one, retired
    ([0x3fff, 0x0000], 20), // documentation
];

fn is_public_v6(ip: Ipv6Addr) -> bool {
    let s = ip.segments();
    let embedded = || Ipv4Addr::from(((s[6] as u32) << 16) | s[7] as u32);
    // "::ffff:10.0.0.1" reaches 10.0.0.1, and so does "64:ff9b::a00:1" where the network translates.
    let mapped = s[..5] == [0; 5] && s[5] == 0xffff;
    let translated = s[..6] == [0x64, 0xff9b, 0, 0, 0, 0];
    if mapped || translated {
        return is_public_v4(embedded());
    }
    // Everything public is global unicast; loopback, link-local, unique local and multicast all lie outside it.
    if s[0] & 0xe000 != 0x2000 {
        return false;
    }
    let bits = ((s[0] as u32) << 16) | s[1] as u32;
    !NOT_PUBLIC_V6.iter().any(|&(first, prefix)| {
        let mask = u32::MAX << (32 - prefix);
        bits & mask == (((first[0] as u32) << 16) | first[1] as u32) & mask
    })
}

/// A host's addresses, or why it has none.
type Lookup = Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>>;

/// The system's resolver.
fn system_lookup(host: String, port: u16) -> Lookup {
    Box::pin(async move { Ok(tokio::net::lookup_host((host, port)).await?.collect()) })
}

/// A resolver that knows one host, at the addresses already checked for it, and nothing else: the connection
/// can't go anywhere a second DNS answer points.
struct Pinned {
    host: String,
    addrs: Vec<SocketAddr>,
}

impl Resolve for Pinned {
    fn resolve(&self, name: Name) -> Resolving {
        let answer: Result<Addrs, Box<dyn std::error::Error + Send + Sync>> =
            if name.as_str() == self.host {
                Ok(Box::new(self.addrs.clone().into_iter()))
            } else {
                Err(format!("{} wasn't checked", name.as_str()).into())
            };
        Box::pin(std::future::ready(answer))
    }
}

/// What a fetch asks for and how much of it to take.
struct Wanted {
    what: &'static str,
    accept: &'static str,
    /// Whether a `Content-Type`, lower-cased and without its parameters, is this kind.
    is: fn(&str) -> bool,
    max_bytes: usize,
}

const PAGE: Wanted = Wanted {
    what: "a web page",
    accept: "text/html, application/xhtml+xml",
    is: |kind| kind == "text/html" || kind == "application/xhtml+xml",
    max_bytes: MAX_PAGE_BYTES,
};

const IMAGE: Wanted = Wanted {
    what: "an image",
    accept: "image/*",
    is: |kind| kind.starts_with("image/"),
    max_bytes: MAX_IMAGE_BYTES,
};

/// A page as fetched.
#[derive(Debug)]
pub struct Page {
    /// Where the page was in the end, after any redirects.
    pub url: Url,
    /// Its text, read as UTF-8; bytes that aren't become "\u{fffd}".
    pub html: String,
}

/// Fetches pages and photos for imports, refusing addresses that aren't on the public internet unless told to
/// allow them.
pub struct Fetcher {
    allow_private: bool,
    lookup: Box<dyn Fn(String, u16) -> Lookup + Send + Sync>,
    page_deadline: Duration,
    image_deadline: Duration,
    /// Exact addresses (address and port) let through as if public: see [`Fetcher::allowing`].
    allowed_addrs: Vec<SocketAddr>,
    /// Addresses a test's own server listens on, let through as if public so the checks around them still run.
    #[cfg(test)]
    trusted: Vec<IpAddr>,
}

impl Fetcher {
    /// A fetcher for the public internet only, or for any address when `allow_private`.
    pub fn new(allow_private: bool) -> Fetcher {
        Fetcher {
            allow_private,
            lookup: Box::new(system_lookup),
            page_deadline: PAGE_DEADLINE,
            image_deadline: IMAGE_DEADLINE,
            allowed_addrs: Vec::new(),
            #[cfg(test)]
            trusted: Vec::new(),
        }
    }

    /// A fetcher as the environment sets it: `IMPORT_ALLOW_PRIVATE=true` (or `1`) allows private addresses.
    pub fn from_env() -> Fetcher {
        let allow = std::env::var("IMPORT_ALLOW_PRIVATE")
            .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "true" | "1"));
        Fetcher::new(allow)
    }

    /// This fetcher, also letting requests through to these exact addresses (address and port) as if they were
    /// public, every other check unchanged: for tests that serve pages from this machine. Another port on the same
    /// address is still refused.
    pub fn allowing(mut self, addrs: Vec<SocketAddr>) -> Fetcher {
        self.allowed_addrs = addrs;
        self
    }

    /// Whether private addresses are allowed (`IMPORT_ALLOW_PRIVATE`).
    pub fn allows_private(&self) -> bool {
        self.allow_private
    }

    /// Fetches the web page at `url`, following redirects.
    pub async fn page(&self, url: &Url) -> Result<Page, FetchError> {
        let (url, body) = self.get(url, &PAGE, self.page_deadline).await?;
        Ok(Page {
            url,
            html: String::from_utf8_lossy(&body).into_owned(),
        })
    }

    /// Fetches the image at `url`, following redirects. The bytes are whatever the site sent as an image: decode
    /// them before trusting them to be one.
    pub async fn image(&self, url: &Url) -> Result<Vec<u8>, FetchError> {
        Ok(self.get(url, &IMAGE, self.image_deadline).await?.1)
    }

    async fn get(
        &self,
        url: &Url,
        wanted: &Wanted,
        deadline: Duration,
    ) -> Result<(Url, Vec<u8>), FetchError> {
        tokio::time::timeout(deadline, self.follow(url.clone(), wanted))
            .await
            .unwrap_or(Err(FetchError::TimedOut))
    }

    /// Whether a request may go to this address.
    fn allowed(&self, ip: IpAddr, port: u16) -> bool {
        #[cfg(test)]
        if self.trusted.contains(&ip) {
            return true;
        }
        self.allow_private
            || is_public(ip)
            || self.allowed_addrs.contains(&SocketAddr::new(ip, port))
    }

    /// Checks one address before anything is sent to it, and resolves its host: the host as the request will name
    /// it, and the addresses the connection may go to (none for a host written as an IP address, which is
    /// connected to as written).
    async fn check(&self, url: &Url) -> Result<(String, Vec<SocketAddr>), FetchError> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(FetchError::BadAddress(
                "only http and https links can be imported".into(),
            ));
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(FetchError::BadAddress(
                "links with a username or password in them can't be imported".into(),
            ));
        }
        if url.as_str().len() > MAX_URL {
            return Err(FetchError::BadAddress("that link is too long".into()));
        }
        let (Some(host), Some(port)) = (url.host(), url.port_or_known_default()) else {
            return Err(FetchError::BadAddress("that link names no site".into()));
        };
        // Ports below 1024 belong to other services (mail, SSH); only the web's own two are fetched from.
        if !self.allow_private && port < 1024 && port != 80 && port != 443 {
            return Err(FetchError::BadAddress(format!(
                "port {port} isn't one mise imports from"
            )));
        }
        let name = host.to_string();
        let not_public = || FetchError::NotPublic { host: name.clone() };
        match host {
            Host::Ipv4(ip) => self
                .allowed(ip.into(), port)
                .then(|| (name.clone(), Vec::new()))
                .ok_or_else(not_public),
            Host::Ipv6(ip) => self
                .allowed(ip.into(), port)
                .then(|| (name.clone(), Vec::new()))
                .ok_or_else(not_public),
            Host::Domain(domain) => {
                let addrs = (self.lookup)(domain.to_string(), port)
                    .await
                    .ok()
                    .filter(|addrs| !addrs.is_empty())
                    .ok_or_else(|| FetchError::NoSuchHost { host: name.clone() })?;
                // One address that isn't public refuses the host: a name that answers with both is not one to
                // pick the good half of.
                if !addrs.iter().all(|a| self.allowed(a.ip(), port)) {
                    return Err(not_public());
                }
                Ok((name.clone(), addrs))
            }
        }
    }

    /// Requests `url` and each address it redirects to in turn, until one answers with what is wanted.
    async fn follow(&self, mut url: Url, wanted: &Wanted) -> Result<(Url, Vec<u8>), FetchError> {
        for _ in 0..=MAX_REDIRECTS {
            let (host, addrs) = self.check(&url).await?;
            let unreachable = || FetchError::Unreachable { host: host.clone() };
            let client = client(&host, addrs).map_err(|_| unreachable())?;
            let mut response = client
                .get(url.clone())
                .header(ACCEPT, wanted.accept)
                .send()
                .await
                .map_err(|_| unreachable())?;
            let status = response.status();
            if status.is_redirection() {
                url = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|l| l.to_str().ok())
                    .and_then(|l| url.join(l).ok())
                    .ok_or(FetchError::Redirects)?;
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Status(status.as_u16()));
            }
            let kind = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(|v| v.trim().to_ascii_lowercase())
                .unwrap_or_default();
            if !(wanted.is)(&kind) {
                return Err(FetchError::WrongType { what: wanted.what });
            }
            let too_large = || FetchError::TooLarge(wanted.max_bytes);
            if response
                .content_length()
                .is_some_and(|n| n > wanted.max_bytes as u64)
            {
                return Err(too_large());
            }
            // The length a site states isn't trusted: the body is counted as it arrives.
            let mut body = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(|_| unreachable())? {
                if body.len() + chunk.len() > wanted.max_bytes {
                    return Err(too_large());
                }
                body.extend_from_slice(&chunk);
            }
            return Ok((url, body));
        }
        Err(FetchError::Redirects)
    }
}

/// A client for one request to `host`, which connects only to `addrs` (or, for a host written as an IP address,
/// to that address). It follows no redirects, uses no proxy (a proxy would do its own resolving, past the checks)
/// and keeps no connection for a later request.
fn client(host: &str, addrs: Vec<SocketAddr>) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(concat!("mise/", env!("CARGO_PKG_VERSION")))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .referer(false)
        .connect_timeout(CONNECT_TIMEOUT)
        .pool_max_idle_per_host(0)
        .dns_resolver(Arc::new(Pinned {
            host: host.to_string(),
            addrs,
        }))
        .build()
}

#[cfg(test)]
mod tests;
