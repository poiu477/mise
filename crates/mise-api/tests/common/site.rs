//! A small web site a test serves on this machine for imports to fetch: fixed answers by path, and a count of what
//! was asked. Tests never fetch real sites.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A site's answers by path: content type and body, which can be changed while it runs.
pub type Pages = Arc<Mutex<HashMap<String, (String, Vec<u8>)>>>;

/// A web server on 127.0.0.1.
pub struct Site {
    pub addr: SocketAddr,
    pub pages: Pages,
    asked: Arc<Mutex<Vec<String>>>,
}

impl Site {
    /// A site answering nothing yet; anything not in `pages` is a 404.
    pub async fn start() -> Site {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let pages: Pages = Arc::default();
        let asked: Arc<Mutex<Vec<String>>> = Arc::default();
        let (served, noted) = (pages.clone(), asked.clone());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (served, noted) = (served.clone(), noted.clone());
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
                    let answer = served.lock().unwrap().get(&path).cloned();
                    let (status, kind, body) = match answer {
                        Some((kind, body)) => ("200 OK", kind, body),
                        None => (
                            "404 Not Found",
                            "text/plain".into(),
                            b"nothing here".to_vec(),
                        ),
                    };
                    let head = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    socket.write_all(head.as_bytes()).await.ok();
                    socket.write_all(&body).await.ok();
                    socket.shutdown().await.ok();
                });
            }
        });
        Site { addr, pages, asked }
    }

    /// Serves `body` at `path` as `kind`, replacing what was there.
    pub fn serve(&self, path: &str, kind: &str, body: impl Into<Vec<u8>>) {
        self.pages
            .lock()
            .unwrap()
            .insert(path.to_string(), (kind.to_string(), body.into()));
    }

    /// The address of `path` here.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    /// How many times `path` was asked for.
    pub fn asked(&self, path: &str) -> usize {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == path)
            .count()
    }
}
