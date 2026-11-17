//! Each thing the fetch refuses, shown refused, against small web servers the tests run themselves on loopback
//! addresses. Nothing here leaves the machine: public addresses are only ever checked, never connected to.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

/// What a test site does with a request for a path.
#[derive(Clone)]
enum Reply {
    /// Answer with a status, headers and a body, then close.
    Send(u16, Vec<(&'static str, String)>, Vec<u8>),
    /// Take the request and never answer.
    Stall,
    /// Send the headers of a page, then never send its body.
    StallAfterHeaders,
}

/// A page of HTML.
fn page(body: &str) -> Reply {
    Reply::Send(
        200,
        vec![("Content-Type", "text/html; charset=utf-8".into())],
        body.as_bytes().to_vec(),
    )
}

/// A redirect to `to`.
fn redirect(to: impl Into<String>) -> Reply {
    Reply::Send(302, vec![("Location", to.into())], Vec::new())
}

/// A web server on a loopback address, which notes the paths asked of it.
struct Site {
    addr: SocketAddr,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Site {
    /// A site on 127.0.0.1 answering these paths; any other path is a 404.
    async fn serving(routes: Vec<(&'static str, Reply)>) -> Site {
        Site::on("127.0.0.1", routes)
            .await
            .expect("127.0.0.1 can be listened on")
    }

    /// A site on `ip`, or None where the machine has no such address (127.0.0.2 on macOS).
    async fn on(ip: &str, routes: Vec<(&'static str, Reply)>) -> Option<Site> {
        let listener = TcpListener::bind((ip, 0)).await.ok()?;
        let addr = listener.local_addr().unwrap();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let routes: Arc<HashMap<&str, Reply>> = Arc::new(routes.into_iter().collect());
        let noted = asked.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (routes, noted) = (routes.clone(), noted.clone());
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(n) if n > 0 => request.extend_from_slice(&buffer[..n]),
                            _ => return,
                        }
                    }
                    let request = String::from_utf8_lossy(&request).into_owned();
                    let path = request.split(' ').nth(1).unwrap_or_default().to_string();
                    noted.lock().unwrap().push(path.clone());
                    let reply = routes.get(path.as_str()).cloned();
                    let (status, headers, body) = match reply {
                        Some(Reply::Send(status, headers, body)) => (status, headers, body),
                        Some(Reply::Stall) => return std::future::pending().await,
                        Some(Reply::StallAfterHeaders) => {
                            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n<html>";
                            socket.write_all(head.as_bytes()).await.ok();
                            return std::future::pending().await;
                        }
                        None => (404, Vec::new(), b"nothing here".to_vec()),
                    };
                    let mut head = format!("HTTP/1.1 {status} Whatever\r\nConnection: close\r\n");
                    for (name, value) in headers {
                        head.push_str(&format!("{name}: {value}\r\n"));
                    }
                    head.push_str("\r\n");
                    // The client may hang up as soon as it has seen enough; that isn't the test's failure.
                    socket.write_all(head.as_bytes()).await.ok();
                    socket.write_all(&body).await.ok();
                    socket.shutdown().await.ok();
                });
            }
        });
        Some(Site { addr, asked })
    }

    /// The address of `path` on this site.
    fn url(&self, path: &str) -> Url {
        Url::parse(&format!("http://{}{path}", self.addr)).unwrap()
    }

    /// The paths asked of this site so far, in order.
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

/// A fetcher for the public internet that also lets requests through to the tests' own sites on 127.0.0.1, so
/// every other check runs as it does in use. Deadlines are short, so a stalled site fails a test quickly.
fn fetcher() -> Fetcher {
    Fetcher {
        trusted: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
        page_deadline: Duration::from_millis(500),
        image_deadline: Duration::from_millis(500),
        ..Fetcher::new(false)
    }
}

/// A fetcher whose names resolve as `answer` says, counting the lookups made.
fn resolving(
    mut fetcher: Fetcher,
    answer: impl Fn(&str, usize) -> Vec<IpAddr> + Send + Sync + 'static,
) -> (Fetcher, Arc<AtomicUsize>) {
    let lookups = Arc::new(AtomicUsize::new(0));
    let count = lookups.clone();
    fetcher.lookup = Box::new(move |host, port| {
        let nth = count.fetch_add(1, Ordering::SeqCst);
        let addrs = answer(&host, nth)
            .into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect();
        Box::pin(std::future::ready(Ok(addrs)))
    });
    (fetcher, lookups)
}

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

/// What checking `url` says, without sending anything.
async fn checked(fetcher: &Fetcher, url: &str) -> Result<(String, Vec<SocketAddr>), FetchError> {
    fetcher.check(&Url::parse(url).unwrap()).await
}

#[test]
fn addresses_on_the_public_internet_are_public() {
    for public in [
        "1.1.1.1",
        "8.8.8.8",
        "93.184.215.14",
        "100.63.255.255",
        "100.128.0.0",
        "172.15.255.255",
        "172.32.0.0",
        "192.0.1.0",
        "198.17.255.255",
        "198.20.0.0",
        "223.255.255.255",
        "2606:4700:4700::1111",
        "2a00:1450:4001:81b::200e",
        "2001:200::1",
        "64:ff9b::808:808",
        "::ffff:8.8.8.8",
    ] {
        assert!(is_public(ip(public)), "{public} is public");
    }
}

#[test]
fn every_kind_of_address_that_is_not_public_is_refused() {
    for (private, what) in [
        ("0.0.0.0", "this network"),
        ("0.255.255.255", "this network"),
        ("10.0.0.1", "a private network"),
        ("10.255.255.255", "a private network"),
        ("100.64.0.1", "carrier-grade NAT"),
        ("100.127.255.255", "carrier-grade NAT"),
        ("127.0.0.1", "loopback"),
        ("127.255.255.254", "loopback"),
        (
            "169.254.169.254",
            "link-local: a cloud host's metadata service",
        ),
        ("169.254.0.1", "link-local"),
        ("172.16.0.1", "a private network"),
        ("172.31.255.255", "a private network"),
        ("192.0.0.8", "protocol assignments"),
        ("192.0.2.1", "documentation"),
        ("192.88.99.1", "6to4 relays"),
        ("192.168.0.1", "a private network"),
        ("192.168.255.255", "a private network"),
        ("198.18.0.1", "benchmarking"),
        ("198.19.255.255", "benchmarking"),
        ("198.51.100.7", "documentation"),
        ("203.0.113.7", "documentation"),
        ("224.0.0.1", "multicast"),
        ("239.255.255.255", "multicast"),
        ("240.0.0.1", "reserved"),
        ("255.255.255.255", "broadcast"),
        ("::", "unspecified"),
        ("::1", "loopback"),
        ("::7f00:1", "IPv4-compatible, retired"),
        ("::ffff:127.0.0.1", "loopback, IPv4-mapped"),
        ("::ffff:10.0.0.1", "a private network, IPv4-mapped"),
        ("::ffff:169.254.169.254", "link-local, IPv4-mapped"),
        ("::ffff:192.168.1.1", "a private network, IPv4-mapped"),
        ("64:ff9b::a00:1", "a private network, through NAT64"),
        ("64:ff9b::7f00:1", "loopback, through NAT64"),
        ("64:ff9b:1::a00:1", "local NAT64"),
        ("100::1", "discard"),
        ("2001::1", "Teredo"),
        ("2001:db8::1", "documentation"),
        ("2002:7f00:1::1", "6to4 carrying loopback"),
        ("2002:808:808::1", "6to4"),
        ("3fff::1", "documentation"),
        ("5f00::1", "segment routing"),
        ("fc00::1", "unique local"),
        ("fd12:3456:789a::1", "unique local"),
        ("fe80::1", "link-local"),
        ("fec0::1", "site-local, retired"),
        ("ff02::1", "multicast"),
    ] {
        assert!(!is_public(ip(private)), "{private} ({what}) is not public");
    }
}

#[tokio::test]
async fn only_http_and_https_links_are_fetched() {
    let f = fetcher();
    for url in [
        "ftp://example.com/recipe.html",
        "file:///etc/passwd",
        "gopher://example.com/",
        "data:text/html,<p>hi</p>",
    ] {
        let refused = checked(&f, url).await.unwrap_err();
        assert!(matches!(refused, FetchError::BadAddress(_)), "{url}");
        assert!(refused.refused(), "{url}");
        assert_eq!(
            refused.to_string(),
            "only http and https links can be imported"
        );
    }
}

#[tokio::test]
async fn a_link_with_a_username_or_password_is_refused() {
    let f = fetcher();
    for url in [
        "http://cook@93.184.215.14/",
        "https://cook:secret@93.184.215.14/recipe",
    ] {
        let refused = checked(&f, url).await.unwrap_err();
        assert!(matches!(refused, FetchError::BadAddress(_)), "{url}");
    }
}

#[tokio::test]
async fn ports_of_other_services_are_refused() {
    let f = fetcher();
    for port in [21, 22, 25, 110, 143, 1023] {
        let url = format!("http://93.184.215.14:{port}/");
        let refused = checked(&f, &url).await.unwrap_err();
        assert!(matches!(refused, FetchError::BadAddress(_)), "{url}");
        assert_eq!(
            refused.to_string(),
            format!("port {port} isn't one mise imports from")
        );
    }
    for url in [
        "http://93.184.215.14/",
        "https://93.184.215.14/",
        "http://93.184.215.14:80/",
        "https://93.184.215.14:443/",
        "http://93.184.215.14:8080/",
        "https://[2606:4700:4700::1111]:8443/",
    ] {
        assert!(checked(&f, url).await.is_ok(), "{url}");
    }
}

#[tokio::test]
async fn a_link_that_is_too_long_is_refused() {
    let url = format!("http://93.184.215.14/{}", "a".repeat(MAX_URL));
    let refused = checked(&fetcher(), &url).await.unwrap_err();
    assert!(matches!(refused, FetchError::BadAddress(_)));
}

#[tokio::test]
async fn an_address_that_is_not_public_is_refused_however_it_is_written() {
    let f = Fetcher::new(false);
    for url in [
        "http://127.0.0.1/",
        "http://127.0.0.1:8080/admin",
        "http://127.1/",
        "http://2130706433/",
        "http://0x7f000001/",
        "http://0x7f.0.0.1/",
        "http://017700000001/",
        "http://0177.0.0.1/",
        "http://0.0.0.0/",
        "http://10.0.0.5/",
        "http://172.16.4.4/",
        "http://192.168.1.1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://100.64.0.1/",
        "http://[::1]/",
        "http://[::1]:5432/",
        "http://[::]/",
        "http://[::ffff:127.0.0.1]/",
        "http://[::ffff:7f00:1]/",
        "http://[::ffff:10.0.0.5]/",
        "http://[64:ff9b::7f00:1]/",
        "http://[fe80::1]/",
        "http://[fd00::1]/",
        "https://[fc00::1]/",
    ] {
        let refused = checked(&f, url).await.unwrap_err();
        assert!(
            matches!(refused, FetchError::NotPublic { .. }),
            "{url}: {refused:?}"
        );
        assert!(refused.refused(), "{url}");
    }
}

#[tokio::test]
async fn a_name_that_resolves_to_an_address_that_is_not_public_is_refused() {
    let (f, _) = resolving(Fetcher::new(false), |host, _| match host {
        "home.test" => vec![ip("192.168.1.20")],
        "loop.test" => vec![ip("127.0.0.1")],
        "six.test" => vec![ip("::1")],
        "mapped.test" => vec![ip("::ffff:10.0.0.1")],
        "metadata.test" => vec![ip("169.254.169.254")],
        // A name with a public address and a private one: all of it is refused, not the private half.
        "both.test" => vec![ip("93.184.215.14"), ip("10.0.0.1")],
        "also-both.test" => vec![ip("10.0.0.1"), ip("93.184.215.14")],
        _ => vec![ip("93.184.215.14"), ip("2606:4700:4700::1111")],
    });
    for host in [
        "home.test",
        "loop.test",
        "six.test",
        "mapped.test",
        "metadata.test",
        "both.test",
        "also-both.test",
    ] {
        let refused = checked(&f, &format!("https://{host}/recipe"))
            .await
            .unwrap_err();
        assert!(
            matches!(&refused, FetchError::NotPublic { host: h } if h == host),
            "{host}: {refused:?}"
        );
        assert_eq!(
            refused.to_string(),
            format!("{host} isn't an address on the public internet")
        );
    }
    let (host, addrs) = checked(&f, "https://recipes.test/soup").await.unwrap();
    assert_eq!(host, "recipes.test");
    assert_eq!(
        addrs,
        [
            "93.184.215.14:443".parse().unwrap(),
            "[2606:4700:4700::1111]:443".parse::<SocketAddr>().unwrap()
        ]
    );
}

#[tokio::test]
async fn the_machine_s_own_name_is_refused() {
    // The system's resolver, as in use: "localhost" is this machine on every system.
    let refused = checked(&Fetcher::new(false), "http://localhost:8080/")
        .await
        .unwrap_err();
    assert!(
        matches!(refused, FetchError::NotPublic { .. }),
        "{refused:?}"
    );
}

#[tokio::test]
async fn a_name_with_no_address_is_not_a_site() {
    let (f, _) = resolving(Fetcher::new(false), |_, _| Vec::new());
    let failed = checked(&f, "https://nowhere.test/").await.unwrap_err();
    assert!(matches!(failed, FetchError::NoSuchHost { .. }));
    assert!(!failed.refused());

    let mut f = Fetcher::new(false);
    f.lookup = Box::new(|_, _| Box::pin(async { Err(std::io::Error::other("no such host")) }));
    let failed = checked(&f, "https://nowhere.test/").await.unwrap_err();
    assert_eq!(failed.to_string(), "there is no site at nowhere.test");
}

#[tokio::test]
async fn a_page_is_fetched_with_its_address() {
    let site = Site::serving(vec![("/soup", page("<title>Leek soup</title>"))]).await;
    let fetched = fetcher().page(&site.url("/soup")).await.unwrap();
    assert_eq!(fetched.html, "<title>Leek soup</title>");
    assert_eq!(fetched.url, site.url("/soup"));
    assert_eq!(site.asked(), ["/soup"]);
}

#[tokio::test]
async fn a_private_address_is_refused_before_anything_is_sent() {
    let site = Site::serving(vec![("/soup", page("soup"))]).await;
    let refused = Fetcher::new(false)
        .page(&site.url("/soup"))
        .await
        .unwrap_err();
    assert!(
        matches!(refused, FetchError::NotPublic { .. }),
        "{refused:?}"
    );
    assert!(site.asked().is_empty(), "the site was asked nothing");
}

#[tokio::test]
async fn private_addresses_are_fetched_when_allowed() {
    let site = Site::serving(vec![("/soup", page("soup"))]).await;
    let f = Fetcher::new(true);
    assert!(f.allows_private());
    assert_eq!(f.page(&site.url("/soup")).await.unwrap().html, "soup");
    // The other checks stay.
    let refused = checked(&f, "file:///etc/passwd").await.unwrap_err();
    assert!(matches!(refused, FetchError::BadAddress(_)));
    let refused = checked(&f, "http://cook:secret@127.0.0.1/")
        .await
        .unwrap_err();
    assert!(matches!(refused, FetchError::BadAddress(_)));
}

#[tokio::test]
async fn redirects_are_followed_to_where_the_page_is() {
    let site = Site::serving(vec![
        ("/old", redirect("/newer")),
        ("/newer", redirect("newest?x=1")),
        ("/newest?x=1", page("here")),
    ])
    .await;
    let fetched = fetcher().page(&site.url("/old")).await.unwrap();
    assert_eq!(fetched.html, "here");
    assert_eq!(fetched.url, site.url("/newest?x=1"));
    assert_eq!(site.asked(), ["/old", "/newer", "/newest?x=1"]);
}

#[tokio::test]
async fn a_redirect_to_an_address_that_is_not_public_is_refused() {
    // Where the machine has a second loopback address, something listens there, to show nothing reaches it.
    let inside = Site::on("127.0.0.2", vec![("/secret", page("the secret"))]).await;
    let inside_url = inside.as_ref().map_or_else(
        || "http://127.0.0.2:8080/secret".to_string(),
        |s| s.url("/secret").to_string(),
    );
    let site = Site::serving(vec![
        ("/to-loopback", redirect(inside_url)),
        ("/to-home", redirect("http://192.168.1.1/admin")),
        (
            "/to-metadata",
            redirect("http://169.254.169.254/latest/meta-data/"),
        ),
        ("/to-six", redirect("http://[::1]/")),
        ("/to-mapped", redirect("http://[::ffff:127.0.0.1]/")),
        ("/to-name", redirect("http://home.test/")),
    ])
    .await;
    let (f, _) = resolving(fetcher(), |_, _| vec![ip("10.0.0.7")]);
    for path in [
        "/to-loopback",
        "/to-home",
        "/to-metadata",
        "/to-six",
        "/to-mapped",
        "/to-name",
    ] {
        let refused = f.page(&site.url(path)).await.unwrap_err();
        assert!(
            matches!(refused, FetchError::NotPublic { .. }),
            "{path}: {refused:?}"
        );
    }
    if let Some(inside) = inside {
        assert!(inside.asked().is_empty(), "nothing reached the inside");
    }
}

#[tokio::test]
async fn a_redirect_to_something_other_than_a_web_address_is_refused() {
    let site = Site::serving(vec![
        ("/to-file", redirect("file:///etc/passwd")),
        ("/to-ssh", redirect("http://93.184.215.14:22/")),
        ("/to-login", redirect("http://admin:admin@93.184.215.14/")),
    ])
    .await;
    for path in ["/to-file", "/to-ssh", "/to-login"] {
        let refused = fetcher().page(&site.url(path)).await.unwrap_err();
        assert!(
            matches!(refused, FetchError::BadAddress(_)),
            "{path}: {refused:?}"
        );
    }
}

#[tokio::test]
async fn redirects_stop_at_the_limit() {
    let site = Site::serving(vec![
        ("/round", redirect("/round")),
        ("/nowhere", Reply::Send(302, Vec::new(), Vec::new())),
    ])
    .await;
    let failed = fetcher().page(&site.url("/round")).await.unwrap_err();
    assert!(matches!(failed, FetchError::Redirects), "{failed:?}");
    assert_eq!(
        site.asked().len(),
        MAX_REDIRECTS + 1,
        "the first request and five redirects"
    );
    // A redirect that names no address leads nowhere.
    let failed = fetcher().page(&site.url("/nowhere")).await.unwrap_err();
    assert!(matches!(failed, FetchError::Redirects), "{failed:?}");
}

#[tokio::test]
async fn the_connection_goes_to_the_address_that_was_checked() {
    let site = Site::serving(vec![("/soup", page("soup"))]).await;
    // A name that answers with the tests' site when it is checked and with another machine's address after:
    // the swap a hostile DNS server makes between the check and the connection.
    let (f, lookups) = resolving(fetcher(), |_, nth| {
        if nth == 0 {
            vec![ip("127.0.0.1")]
        } else {
            vec![ip("127.0.0.2")]
        }
    });
    let url = Url::parse(&format!("http://rebind.test:{}/soup", site.addr.port())).unwrap();
    let fetched = f.page(&url).await.unwrap();
    assert_eq!(fetched.html, "soup");
    assert_eq!(site.asked(), ["/soup"]);
    assert_eq!(
        lookups.load(Ordering::SeqCst),
        1,
        "the name was resolved once: the connection didn't ask again"
    );
}

#[tokio::test]
async fn a_client_reaches_only_the_host_it_was_made_for() {
    let pinned = Pinned {
        host: "recipes.test".into(),
        addrs: vec!["93.184.215.14:443".parse().unwrap()],
    };
    let known: Vec<SocketAddr> = pinned
        .resolve("recipes.test".parse().unwrap())
        .await
        .unwrap()
        .collect();
    assert_eq!(known, ["93.184.215.14:443".parse().unwrap()]);
    assert!(
        pinned
            .resolve("elsewhere.test".parse().unwrap())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_error_from_the_site_is_reported_with_its_status() {
    let site = Site::serving(vec![(
        "/broken",
        Reply::Send(500, Vec::new(), b"oops".to_vec()),
    )])
    .await;
    for (path, status) in [("/gone", 404), ("/broken", 500)] {
        let failed = fetcher().page(&site.url(path)).await.unwrap_err();
        assert!(matches!(failed, FetchError::Status(s) if s == status));
        assert!(!failed.refused());
    }
}

#[tokio::test]
async fn an_answer_of_the_wrong_kind_is_refused() {
    let png = vec![("Content-Type", "image/png".to_string())];
    let site = Site::serving(vec![
        (
            "/a.pdf",
            Reply::Send(
                200,
                vec![("Content-Type", "application/pdf".into())],
                b"%PDF".to_vec(),
            ),
        ),
        ("/untyped", Reply::Send(200, Vec::new(), b"<html>".to_vec())),
        ("/photo.png", Reply::Send(200, png, b"\x89PNG".to_vec())),
        ("/page", page("<html>")),
        (
            "/xhtml",
            Reply::Send(
                200,
                vec![("Content-Type", "Application/XHTML+XML".into())],
                b"<html/>".to_vec(),
            ),
        ),
    ])
    .await;
    let f = fetcher();
    for path in ["/a.pdf", "/untyped", "/photo.png"] {
        let failed = f.page(&site.url(path)).await.unwrap_err();
        assert!(
            matches!(failed, FetchError::WrongType { what: "a web page" }),
            "{path}: {failed:?}"
        );
    }
    assert_eq!(f.page(&site.url("/xhtml")).await.unwrap().html, "<html/>");
    assert_eq!(
        f.image(&site.url("/photo.png")).await.unwrap(),
        b"\x89PNG".to_vec()
    );
    for path in ["/page", "/a.pdf", "/untyped"] {
        let failed = f.image(&site.url(path)).await.unwrap_err();
        assert!(
            matches!(failed, FetchError::WrongType { what: "an image" }),
            "{path}: {failed:?}"
        );
    }
}

#[tokio::test]
async fn an_answer_over_the_size_limit_is_refused() {
    let html = vec![("Content-Type", "text/html".to_string())];
    let mut stated = html.clone();
    stated.push(("Content-Length", (MAX_PAGE_BYTES + 1).to_string()));
    let site = Site::serving(vec![
        // A length stated up front is refused without reading the body.
        ("/stated", Reply::Send(200, stated, Vec::new())),
        // Without one, the body is counted as it arrives.
        (
            "/unstated",
            Reply::Send(200, html.clone(), vec![b'a'; MAX_PAGE_BYTES + 1]),
        ),
        (
            "/just-fits",
            Reply::Send(200, html, vec![b'a'; MAX_PAGE_BYTES]),
        ),
    ])
    .await;
    let f = Fetcher {
        page_deadline: Duration::from_secs(20),
        ..fetcher()
    };
    for path in ["/stated", "/unstated"] {
        let failed = f.page(&site.url(path)).await.unwrap_err();
        assert!(
            matches!(failed, FetchError::TooLarge(MAX_PAGE_BYTES)),
            "{path}: {failed:?}"
        );
        assert_eq!(failed.to_string(), "what the site sent is larger than 5 MB");
    }
    let fits = f.page(&site.url("/just-fits")).await.unwrap();
    assert_eq!(fits.html.len(), MAX_PAGE_BYTES);
}

#[tokio::test]
async fn a_site_that_stalls_runs_out_of_time() {
    let site = Site::serving(vec![
        ("/silent", Reply::Stall),
        ("/half", Reply::StallAfterHeaders),
        ("/slow-redirect", redirect("/silent")),
    ])
    .await;
    for path in ["/silent", "/half", "/slow-redirect"] {
        let started = std::time::Instant::now();
        let failed = fetcher().page(&site.url(path)).await.unwrap_err();
        assert!(matches!(failed, FetchError::TimedOut), "{path}: {failed:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{path} gave up at the deadline"
        );
    }
    let failed = fetcher().image(&site.url("/silent")).await.unwrap_err();
    assert!(matches!(failed, FetchError::TimedOut), "{failed:?}");
}

#[tokio::test]
async fn a_site_that_is_not_there_does_not_answer() {
    // A port that was free a moment ago: nothing listens on it now.
    let gone = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = gone.local_addr().unwrap().port();
    drop(gone);
    let url = Url::parse(&format!("http://127.0.0.1:{port}/soup")).unwrap();
    // Windows tries a closed port for two seconds before saying so.
    let f = Fetcher {
        page_deadline: Duration::from_secs(15),
        ..fetcher()
    };
    let failed = f.page(&url).await.unwrap_err();
    assert!(
        matches!(failed, FetchError::Unreachable { .. }),
        "{failed:?}"
    );
    assert_eq!(failed.to_string(), "127.0.0.1 didn't answer");
}

#[test]
fn the_environment_allows_private_addresses_only_when_it_says_so() {
    // One test sets the variable, so no other reads it half way.
    for (value, allowed) in [
        (Some("true"), true),
        (Some("TRUE"), true),
        (Some("1"), true),
        (Some("false"), false),
        (Some("yes please"), false),
        (Some(""), false),
        (None, false),
    ] {
        // SAFETY: no other test in this crate reads or writes the environment.
        unsafe {
            match value {
                Some(v) => std::env::set_var("IMPORT_ALLOW_PRIVATE", v),
                None => std::env::remove_var("IMPORT_ALLOW_PRIVATE"),
            }
        }
        assert_eq!(Fetcher::from_env().allows_private(), allowed, "{value:?}");
    }
}
