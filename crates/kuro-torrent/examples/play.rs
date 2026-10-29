//! Opens a magnet with the engine and reads the first bytes and a later range from the local address, as the player
//! does. `cargo run -p kuro-torrent --example play -- '<magnet>' <cache dir>`
use std::time::Instant;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn get(address: &str, range: &str) -> String {
    let rest = address.strip_prefix("http://").unwrap();
    let (host, path) = rest.split_once('/').unwrap();
    let mut conn = tokio::net::TcpStream::connect(host).await.unwrap();
    conn.write_all(format!("GET /{path} HTTP/1.1\r\nHost: {host}\r\nRange: {range}\r\n\r\n").as_bytes()).await.unwrap();
    let mut buf = vec![0u8; 2048];
    let mut got = Vec::new();
    while got.len() < 2048 {
        let n = conn.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        got.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&got);
    text.split("\r\n\r\n").next().unwrap_or_default().to_owned()
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let magnet = args.next().expect("a magnet link");
    let cache = std::path::PathBuf::from(args.next().expect("a cache folder"));
    let started = Instant::now();
    let address = kuro_torrent::open(&magnet, &cache).await.expect("the torrent opens");
    println!("opened in {:.1}s: {address}", started.elapsed().as_secs_f32());
    let started = Instant::now();
    println!("--- first bytes ({:.1}s)\n{}", started.elapsed().as_secs_f32(), get(&address, "bytes=0-1023").await);
    let started = Instant::now();
    println!("--- a later range ({:.1}s)\n{}", started.elapsed().as_secs_f32(), get(&address, "bytes=50000000-50001023").await);
    kuro_torrent::close().await;
    println!("closed; files left: {}", std::fs::read_dir(cache.join("files")).map(|d| d.count()).unwrap_or(0));
}
