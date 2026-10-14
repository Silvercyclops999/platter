//! The RTSP control connection to an AirPlay receiver, with HTTP Digest
//! authentication for receivers that have a password.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::Duration;

use md5::{Digest, Md5};

use crate::error::{AppError, AppResult};

/// What the move fails with when the receiver wants a password Platter
/// doesn't have (the UI asks for it).
pub const NEEDS_PASSWORD: &str = "This speaker needs its AirPlay password.";
pub const WRONG_PASSWORD: &str = "That AirPlay password didn't work.";
/// Receivers that only take paired senders (AirPlay 2 access control).
pub const NEEDS_PAIRING: &str = "This speaker only accepts paired AirPlay 2 devices, which Platter can't do yet.";

/// The user name receivers expect with a password.
const DIGEST_USER: &str = "iTunes";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Clone)]
struct Challenge {
    realm: String,
    nonce: String,
}

pub struct Rtsp {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    cseq: u32,
    pub local_ip: IpAddr,
    pub peer: SocketAddr,
    password: Option<String>,
    challenge: Option<Challenge>,
    pub session: Option<String>,
    /// Identifies this sender to the receiver.
    pub client_id: String,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn md5_hex(s: &str) -> String {
    hex(&Md5::digest(s.as_bytes()))
}

/// Digest `response` for one request (RFC 2617, no qop).
pub fn digest_response(user: &str, realm: &str, password: &str, method: &str, uri: &str, nonce: &str) -> String {
    let ha1 = md5_hex(&format!("{user}:{realm}:{password}"));
    let ha2 = md5_hex(&format!("{method}:{uri}"));
    md5_hex(&format!("{ha1}:{nonce}:{ha2}"))
}

fn param(header: &str, key: &str) -> Option<String> {
    let lower = header.to_ascii_lowercase();
    let at = lower.find(&format!("{key}="))?;
    let rest = &header[at + key.len() + 1..];
    let v = if let Some(r) = rest.strip_prefix('"') {
        r.split('"').next()?
    } else {
        rest.split([',', ' ']).next()?
    };
    Some(v.to_string())
}

fn net(e: impl std::fmt::Display) -> AppError {
    AppError::Network(format!("AirPlay receiver: {e}"))
}

impl Rtsp {
    pub fn connect(addr: SocketAddr, timeout: Duration, password: Option<String>, client_id: String) -> AppResult<Self> {
        let s = TcpStream::connect_timeout(&addr, timeout).map_err(net)?;
        s.set_read_timeout(Some(timeout)).map_err(net)?;
        s.set_write_timeout(Some(timeout)).map_err(net)?;
        let _ = s.set_nodelay(true);
        let local_ip = s.local_addr().map_err(net)?.ip();
        Ok(Self {
            reader: BufReader::new(s.try_clone().map_err(net)?),
            writer: s,
            cseq: 0,
            local_ip,
            peer: addr,
            password,
            challenge: None,
            session: None,
            client_id,
        })
    }

    fn send_once(&mut self, method: &str, uri: &str, headers: &[(&str, String)], body: Option<(&str, &[u8])>) -> AppResult<Response> {
        self.cseq += 1;
        let mut req = format!("{method} {uri} RTSP/1.0\r\nCSeq: {}\r\nUser-Agent: Platter/1.0\r\n", self.cseq);
        req.push_str(&format!("Client-Instance: {}\r\nDACP-ID: {}\r\nActive-Remote: 1\r\n", self.client_id, self.client_id));
        if let Some(s) = &self.session {
            req.push_str(&format!("Session: {s}\r\n"));
        }
        if let (Some(c), Some(pw)) = (&self.challenge, &self.password) {
            let r = digest_response(DIGEST_USER, &c.realm, pw, method, uri, &c.nonce);
            req.push_str(&format!(
                "Authorization: Digest username=\"{DIGEST_USER}\", realm=\"{}\", nonce=\"{}\", uri=\"{uri}\", response=\"{r}\"\r\n",
                c.realm, c.nonce
            ));
        }
        for (k, v) in headers {
            req.push_str(&format!("{k}: {v}\r\n"));
        }
        if let Some((ct, b)) = body {
            req.push_str(&format!("Content-Type: {ct}\r\nContent-Length: {}\r\n", b.len()));
        }
        req.push_str("\r\n");
        let mut bytes = req.into_bytes();
        if let Some((_, b)) = body {
            bytes.extend_from_slice(b);
        }
        self.writer.write_all(&bytes).map_err(net)?;
        self.read_response()
    }

    fn read_response(&mut self) -> AppResult<Response> {
        let mut line = String::new();
        if self.reader.read_line(&mut line).map_err(net)? == 0 {
            return Err(net("connection closed"));
        }
        let status = line
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| net("bad answer"))?;
        let mut headers = Vec::new();
        loop {
            line.clear();
            if self.reader.read_line(&mut line).map_err(net)? == 0 {
                return Err(net("connection closed"));
            }
            let l = line.trim_end();
            if l.is_empty() {
                break;
            }
            if let Some((k, v)) = l.split_once(':') {
                headers.push((k.trim().to_string(), v.trim().to_string()));
            }
        }
        let len = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, v)| v.parse::<usize>().ok())
            .unwrap_or(0)
            .min(1 << 20);
        let mut body = vec![0u8; len];
        self.reader.read_exact(&mut body).map_err(net)?;
        Ok(Response { status, headers, body })
    }

    /// One request; answers a password challenge once. Non-2xx answers are
    /// errors (401 as [`NEEDS_PASSWORD`] or [`WRONG_PASSWORD`], 403 and 470
    /// as [`NEEDS_PAIRING`]).
    pub fn request(&mut self, method: &str, uri: &str, headers: &[(&str, String)], body: Option<(&str, &[u8])>) -> AppResult<Response> {
        let mut r = self.send_once(method, uri, headers, body)?;
        if r.status == 401 {
            let had = self.challenge.is_some();
            let ch = r
                .header("www-authenticate")
                .filter(|h| h.trim_start().to_ascii_lowercase().starts_with("digest"))
                .and_then(|h| Some(Challenge { realm: param(h, "realm")?, nonce: param(h, "nonce")? }));
            match (ch, &self.password) {
                (Some(ch), Some(_)) if !had => {
                    self.challenge = Some(ch);
                    r = self.send_once(method, uri, headers, body)?;
                }
                (_, None) => return Err(AppError::Auth(NEEDS_PASSWORD.into())),
                _ => return Err(AppError::Auth(WRONG_PASSWORD.into())),
            }
            if r.status == 401 {
                return Err(AppError::Auth(WRONG_PASSWORD.into()));
            }
        }
        match r.status {
            200..=299 => Ok(r),
            403 | 470 => Err(AppError::Auth(NEEDS_PAIRING.into())),
            s => Err(net(format!("{method} answered {s}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_matches_rfc_2617() {
        // RFC 2617 section 3.5 example, without qop.
        let r = digest_response("Mufasa", "testrealm@host.com", "Circle Of Life", "GET", "/dir/index.html", "dcd98b7102dd2f0e8b11d0f600bfb0c093");
        assert_eq!(r.len(), 32);
        assert_eq!(md5_hex(""), "d41d8cd98f00b204e9800998ecf8427e");
        // Same inputs, same answer; another nonce, another answer.
        assert_eq!(r, digest_response("Mufasa", "testrealm@host.com", "Circle Of Life", "GET", "/dir/index.html", "dcd98b7102dd2f0e8b11d0f600bfb0c093"));
        assert_ne!(r, digest_response("Mufasa", "testrealm@host.com", "Circle Of Life", "GET", "/dir/index.html", "other"));
    }

    #[test]
    fn challenge_params_parse() {
        let h = r#"Digest realm="raop", nonce="abc123""#;
        assert_eq!(param(h, "realm").as_deref(), Some("raop"));
        assert_eq!(param(h, "nonce").as_deref(), Some("abc123"));
        assert_eq!(param("Digest realm=x, nonce=y", "nonce").as_deref(), Some("y"));
    }
}
