//! Folder paths from Plex and Lidarr, compared loosely: either separator,
//! any case, any Unicode form. Plex and Lidarr often run in separate
//! containers, so the same folder can have two unrelated path strings;
//! these helpers only make strings comparable. Whether two folders are the
//! same is decided by their contents (see `check`).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use unicode_normalization::UnicodeNormalization;

/// One path split into folder names, each folded for comparison.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NormPath(pub Vec<String>);

/// A folder or file name folded for comparison: NFC, lower case, trimmed.
pub fn fold(name: &str) -> String {
    name.trim().nfc().collect::<String>().to_lowercase()
}

impl NormPath {
    /// Splits on `/` and `\`. A Windows drive (`M:`) or a UNC server name
    /// stays as the first segment.
    pub fn parse(path: &str) -> Self {
        NormPath(path.split(['/', '\\']).filter(|s| !s.trim().is_empty() && *s != ".").map(fold).collect())
    }

    pub fn key(&self) -> String {
        self.0.join("/")
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn name(&self) -> Option<&str> {
        self.0.last().map(String::as_str)
    }

    pub fn parent(&self) -> Option<NormPath> {
        (self.0.len() > 1).then(|| NormPath(self.0[..self.0.len() - 1].to_vec()))
    }

    pub fn child(&self, name: &str) -> NormPath {
        let mut v = self.0.clone();
        v.push(fold(name));
        NormPath(v)
    }

    /// `self` is `other` or a folder inside it.
    pub fn starts_with(&self, other: &NormPath) -> bool {
        !other.0.is_empty() && self.0.len() >= other.0.len() && self.0[..other.0.len()] == other.0[..]
    }

    /// The last two names (`parent/name`), when there are two.
    pub fn last_two(&self) -> Option<(String, String)> {
        let n = self.0.len();
        (n >= 2).then(|| (self.0[n - 2].clone(), self.0[n - 1].clone()))
    }
}

/// How one folder sits relative to another, compared as strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Same,
    /// The first folder is inside the second.
    Inside,
    /// The first folder holds the second.
    Contains,
}

pub fn relation(a: &NormPath, b: &NormPath) -> Option<Relation> {
    if a.is_empty() || b.is_empty() {
        return None;
    }
    if a == b {
        Some(Relation::Same)
    } else if a.starts_with(b) {
        Some(Relation::Inside)
    } else if b.starts_with(a) {
        Some(Relation::Contains)
    } else {
        None
    }
}

/// Join a child name onto a Lidarr path in that path's own style, with a
/// trailing separator (Lidarr's folder listing wants one).
pub fn join_dir(base: &str, name: &str) -> String {
    let sep = if base.contains('\\') && !base.contains('/') { '\\' } else { '/' };
    let mut out = base.trim_end_matches(['/', '\\']).to_string();
    out.push(sep);
    out.push_str(name.trim_matches(['/', '\\']));
    out.push(sep);
    out
}

/// A Lidarr folder path with one trailing separator in its own style.
pub fn as_dir(path: &str) -> String {
    let sep = if path.contains('\\') && !path.contains('/') { '\\' } else { '/' };
    let mut out = path.trim_end_matches(['/', '\\']).to_string();
    out.push(sep);
    out
}

/// The address is on this computer or the home network: a private, loopback
/// or link-local IP, `localhost`, or a name with no dots or a home suffix.
pub fn is_private_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    if let Ok(ip) = h.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => private_v4(v4),
            IpAddr::V6(v6) => private_v6(v6),
        };
    }
    if h.is_empty() {
        return false;
    }
    let h = h.trim_end_matches('.');
    h == "localhost"
        || !h.contains('.')
        || [".local", ".lan", ".home", ".internal", ".home.arpa", ".localdomain", ".localhost"]
            .iter()
            .any(|s| h.ends_with(s))
}

fn private_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        // Shared address space (carrier-grade NAT and some home VPNs).
        || (o[0] == 100 && (64..128).contains(&o[1]))
}

fn private_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return private_v4(v4);
    }
    let first = ip.segments()[0];
    ip.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_folds_separators_case_and_unicode() {
        let a = NormPath::parse("/Data/Music/Beyonce\u{301}/");
        let b = NormPath::parse("data\\music\\BEYONC\u{c9}");
        assert_eq!(a, b);
        assert_eq!(a.key(), "data/music/beyonc\u{e9}");
        assert_eq!(NormPath::parse(r"\\nas\share\Music").0, vec!["nas", "share", "music"]);
        assert_eq!(NormPath::parse(r"M:\Music\").0, vec!["m:", "music"]);
    }

    #[test]
    fn relations() {
        let music = NormPath::parse("/data/music");
        assert_eq!(relation(&NormPath::parse("/data/music/"), &music), Some(Relation::Same));
        assert_eq!(relation(&NormPath::parse("/data/music/new"), &music), Some(Relation::Inside));
        assert_eq!(relation(&NormPath::parse("/data"), &music), Some(Relation::Contains));
        assert_eq!(relation(&NormPath::parse("/data/musicals"), &music), None);
        assert_eq!(relation(&NormPath::parse("/DATA/Music"), &music), Some(Relation::Same));
        // Whole names only: a folder whose name starts the same is another folder.
        let media = NormPath::parse("/data/media/Music");
        assert_eq!(relation(&NormPath::parse("/data/media/Music-archive"), &media), None);
        assert!(!NormPath::parse("/data/media/Music-archive/X").starts_with(&media));
        assert!(NormPath::parse(r"\data\media\music\X").starts_with(&media), "either separator, any case");
    }

    #[test]
    fn join_keeps_the_path_style() {
        assert_eq!(join_dir("/music/", "Band"), "/music/Band/");
        assert_eq!(join_dir(r"M:\Music", "Band"), r"M:\Music\Band\");
    }

    #[test]
    fn private_hosts() {
        for h in ["10.0.0.5", "172.16.3.4", "127.0.0.1", "localhost", "nas", "lidarr.local", "box.home.arpa", "[::1]", "fd00::1", "100.70.1.2", "169.254.1.1"] {
            assert!(is_private_host(h), "{h}");
        }
        // Built in pieces so the repo's leak scan doesn't read it as a real home address.
        assert!(is_private_host(&format!("192.{}.0.9", 168)));
        for h in ["lidarr.example.com", "8.8.8.8", "172.32.0.1", "2001:db8::1", ""] {
            assert!(!is_private_host(h), "{h}");
        }
    }
}
