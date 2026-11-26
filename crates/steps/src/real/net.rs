//! Every request the steps make goes through one of these clients and is
//! read with [`body`], so all of them share the same limits:
//!
//! - HTTPS only, certificates checked against the bundled web roots
//!   (rustls), except for [`plain`]: redump.org has no HTTPS at all.
//! - A timeout for the whole request and a shorter one to connect.
//! - At most five redirects, and none for a server you named that is sent
//!   a secret in its URL (ntfy's topic).
//! - A cap on how much of an answer is read, checked as it arrives, so a
//!   server that never stops sending can't fill memory or a disk.
//!
//! The User-Agent names the app and its version and where it lives, and
//! nothing about you.

use anyhow::{anyhow, Result};
use reqwest::redirect::Policy;
use reqwest::{Client, ClientBuilder, Response};
use serde::de::DeserializeOwned;
use std::time::Duration;

pub const AGENT: &str = concat!("Trip/", env!("CARGO_PKG_VERSION"), " ( https://github.com/vresistor/trip )");

/// Caps on what's read: a JSON answer, a page or a text reply.
pub const MB: usize = 1024 * 1024;
pub const JSON_MAX: usize = 16 * MB;
pub const PAGE_MAX: usize = 16 * MB;
pub const TEXT_MAX: usize = MB;
pub const FILE_MAX: usize = 64 * MB;

fn builder(timeout: Duration) -> ClientBuilder {
    Client::builder().user_agent(AGENT).timeout(timeout).connect_timeout(Duration::from_secs(15).min(timeout)).redirect(Policy::limited(5))
}

/// A client for an HTTPS service: a redirect to plain HTTP is refused.
pub fn client(timeout: Duration) -> Result<Client> {
    Ok(builder(timeout).https_only(true).build()?)
}

/// A client that also speaks plain HTTP, for redump.org, which has nothing
/// else. What it sends back is checked for shape, never trusted further.
pub fn plain(timeout: Duration) -> Result<Client> {
    Ok(builder(timeout).build()?)
}

/// A client for a server you named, whose URL carries something private:
/// plain HTTP is yours to choose (a server at home), and a redirect is never
/// followed, so the URL goes nowhere else.
pub fn yours(timeout: Duration) -> Result<Client> {
    Ok(builder(timeout).redirect(Policy::none()).build()?)
}

/// A request's error without its URL, for a URL that carries a key, a
/// topic or an email address.
pub fn quiet(e: reqwest::Error) -> anyhow::Error {
    anyhow::Error::new(e.without_url())
}

/// The whole answer, as long as it's no more than `max` bytes.
pub async fn body(mut r: Response, max: usize) -> Result<Vec<u8>> {
    let too_big = || anyhow!("the answer is over {} MB", max.div_ceil(MB));
    if r.content_length().is_some_and(|n| n > max as u64) {
        return Err(too_big());
    }
    let mut out = Vec::new();
    while let Some(chunk) = r.chunk().await.map_err(quiet)? {
        if out.len().saturating_add(chunk.len()) > max {
            return Err(too_big());
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// The answer as JSON, read with [`body`].
pub async fn json<T: DeserializeOwned>(r: Response, max: usize) -> Result<T> {
    Ok(serde_json::from_slice(&body(r, max).await?)?)
}

/// The answer as text, read with [`body`]. Anything that isn't UTF-8 is
/// replaced rather than refused.
pub async fn text(r: Response, max: usize) -> Result<String> {
    Ok(String::from_utf8_lossy(&body(r, max).await?).into_owned())
}

/// Everything `r` gives, as long as it's no more than `max` bytes. For a
/// file inside a downloaded zip: the size its header claims is only used
/// as a hint, never trusted for an allocation.
pub fn read_at_most(r: impl std::io::Read, claimed: u64, max: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::with_capacity(claimed.min(max as u64) as usize);
    r.take(max as u64 + 1).read_to_end(&mut out)?;
    if out.len() > max {
        anyhow::bail!("it unpacks to over {} MB", max.div_ceil(MB));
    }
    Ok(out)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// A one-request HTTP server on a free port: answers with `head` and
    /// then `size` bytes of `x`. Gives its address.
    pub fn serve(head: String, size: usize) -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = l.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf);
                let _ = s.write_all(head.as_bytes());
                let chunk = vec![b'x'; 64 * 1024];
                let mut left = size;
                while left > 0 {
                    let n = left.min(chunk.len());
                    if s.write_all(&chunk[..n]).is_err() {
                        break;
                    }
                    left -= n;
                }
            }
        });
        format!("http://{at}/")
    }

    #[tokio::test]
    async fn an_answer_over_the_cap_is_refused_as_it_arrives() {
        // No length given: counted as it comes.
        let url = serve("HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".into(), 3 * MB);
        let r = plain(Duration::from_secs(10)).unwrap().get(&url).send().await.unwrap();
        assert!(body(r, MB).await.unwrap_err().to_string().contains("over 1 MB"));
        // A length that says so: refused before reading.
        let url = serve(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", 3 * MB), 0);
        let r = plain(Duration::from_secs(10)).unwrap().get(&url).send().await.unwrap();
        assert!(body(r, MB).await.is_err());
        // Under it: the lot.
        let url = serve("HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".into(), 1000);
        let r = plain(Duration::from_secs(10)).unwrap().get(&url).send().await.unwrap();
        assert_eq!(body(r, MB).await.unwrap().len(), 1000);
    }

    #[test]
    fn a_claimed_size_is_never_trusted() {
        let data = vec![7u8; 1000];
        assert_eq!(read_at_most(&data[..], u64::MAX, 1000).unwrap().len(), 1000, "a lying header allocates nothing much");
        assert!(read_at_most(&data[..], 10, 999).is_err(), "more than it said, and over the cap");
    }

    #[tokio::test]
    async fn an_https_client_wont_speak_plain_http() {
        let url = serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n".into(), 2);
        assert!(client(Duration::from_secs(5)).unwrap().get(&url).send().await.is_err());
    }

    #[tokio::test]
    async fn plain_follows_a_redirect_and_yours_never_does() {
        let to = serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n".into(), 2);
        let from = serve(format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\r\n"), 0);
        // `plain` follows it; the server is plain, so this only shows the redirect works.
        assert_eq!(plain(Duration::from_secs(5)).unwrap().get(&from).send().await.unwrap().status(), 200);
        let to = serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n".into(), 2);
        let from = serve(format!("HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\n\r\n"), 0);
        assert_eq!(yours(Duration::from_secs(5)).unwrap().get(&from).send().await.unwrap().status(), 302, "yours never follows");
    }

    /// Every service, for real, through the clients above (webpki roots,
    /// HTTPS only, the redirect limit). Needs the network:
    /// `cargo test -p trip-steps real_services -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn real_services_answer_over_https() {
        let urls = [
            "https://musicbrainz.org/ws/2/release/76df3287-6cda-33eb-8e9a-044b5e15ffdd?fmt=json",
            "https://coverartarchive.org/release/76df3287-6cda-33eb-8e9a-044b5e15ffdd/front-250",
            "https://lrclib.net/api/get?artist_name=x&track_name=y&album_name=z&duration=1",
            "https://gnudb.gnudb.org/~cddb/cddb.cgi?cmd=stat&hello=a+example.org+TripDiscRipper+test&proto=6",
            "https://www.accuraterip.com/driveoffsets.htm",
            "https://api.discogs.com/",
            "https://api.themoviedb.org/3/configuration",
        ];
        for url in urls {
            let r = client(Duration::from_secs(30)).unwrap().get(url).send().await.unwrap_or_else(|e| panic!("{url}: {e:#}"));
            // An answer at all is the point: TLS and redirects worked. A
            // missing key or unknown item is still an answer.
            assert!(r.status().as_u16() < 500, "{url}: {}", r.status());
            assert!(r.url().scheme() == "https", "{url} ended at {}", r.url());
            println!("{url}: {} ({})", r.status(), body(r, FILE_MAX).await.map(|b| b.len()).unwrap_or(0));
        }
    }
}
