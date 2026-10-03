#![allow(dead_code)]
use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub fn free_addr(ip: &str) -> SocketAddr {
    let l = std::net::TcpListener::bind(format!("{ip}:0")).unwrap();
    l.local_addr().unwrap()
}

pub async fn wait_listening(addr: SocketAddr) {
    for _ in 0..200 {
        if TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{addr} never started listening");
}

pub async fn accepts(addr: SocketAddr) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_secs(1), TcpStream::connect(addr)).await,
        Ok(Ok(_))
    )
}

/// Sends raw bytes, reads until EOF (or 5s), returns (status, whole response).
/// Status 0 means the server closed or answered nothing parseable.
pub async fn raw(addr: SocketAddr, req: &[u8]) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.expect("connect");
    let _ = s.write_all(req).await;
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf).to_string();
    (status_of(&text), text)
}

pub fn status_of(text: &str) -> u16 {
    text.split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .filter(|_| text.starts_with("HTTP/"))
        .unwrap_or(0)
}

pub fn statuses(text: &str) -> Vec<u16> {
    text.match_indices("HTTP/1.")
        .filter_map(|(i, _)| text[i..].split_whitespace().nth(1)?.parse().ok())
        .collect()
}
