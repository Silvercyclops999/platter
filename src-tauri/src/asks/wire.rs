//! How a request travels between someone the server is shared with and the
//! owner's Platter.
//!
//! Both sides already hold one secret nobody else has: the access token
//! Plex gave that person for this server. The person's Platter streams with
//! it, and plex.tv lists it for the owner next to the share. Neither side
//! ever sends it. Each derives the same 32-byte key from it (HMAC with the
//! server's machine id), and two relay channels come from that key: `up`
//! for the person's messages, `down` for the owner's answers. So the relay
//! sees random topics and ciphertext, and a message that opens on someone's
//! `up` channel can only have come from them (or from the owner).
//!
//! Messages carry a time and an id, and both sides treat every message as
//! idempotent: a request sent twice is one request, a status seen twice is
//! one status, and anything older than the last one seen is dropped.

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::plex::token::Token;
use crate::remote::peer::auth::hmac_sha256;
use crate::social::listen::seal::{self, Channel, Rejected};

const LABEL: &str = "platter-requests-v1";

/// The two channels for one person on one server.
#[derive(Debug, Clone)]
pub struct Pair {
    pub up: Channel,
    pub down: Channel,
}

impl Pair {
    pub fn new(server_token: &Token, machine_id: &str) -> Self {
        let secret = hmac_sha256(server_token.expose().as_bytes(), format!("{LABEL} {machine_id}").as_bytes());
        Self { up: Channel::new(&secret, "up"), down: Channel::new(&secret, "down") }
    }
}

/// What was asked for: an album, or a song (which comes with its album or
/// single).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskItem {
    /// The MusicBrainz release group, when the asking side knows it.
    #[serde(default)]
    pub release_group_id: Option<String>,
    pub title: String,
    pub artist: String,
    #[serde(default)]
    pub artist_mbid: Option<String>,
    /// For a song: the album it's on, when known.
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub song: bool,
    #[serde(default)]
    pub isrc: Option<String>,
    /// Cover art as the catalogue gave it (a web address, never a server
    /// path). Kept on the asking side only: the owner's side shows the
    /// release group's own cover, never an address someone else sent.
    #[serde(default)]
    pub thumb: Option<String>,
}

impl AskItem {
    /// Within what the relay takes, and nothing silly.
    pub fn sane(&self) -> bool {
        let short = |s: &str, n: usize| s.len() <= n;
        !self.title.trim().is_empty()
            && !self.artist.trim().is_empty()
            && short(&self.title, 300)
            && short(&self.artist, 300)
            && self.album.as_deref().is_none_or(|a| short(a, 300))
            && self.release_group_id.as_deref().is_none_or(|r| short(r, 64))
            && self.artist_mbid.as_deref().is_none_or(|r| short(r, 64))
            && self.isrc.as_deref().is_none_or(|r| short(r, 32))
            && self.thumb.as_deref().is_none_or(|t| short(t, 600) && (t.starts_with("https://") || t.starts_with("/ext/https://")))
    }

    /// The album name shown for it (a song's album, else its own title).
    pub fn album_title(&self) -> &str {
        match (&self.album, self.song) {
            (Some(a), true) if !a.trim().is_empty() => a,
            _ => &self.title,
        }
    }
}

/// From the person to the owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Up {
    /// "Can I ask?" The owner answers with [`Down::Policy`].
    Hello { at: u64 },
    Ask { id: String, item: AskItem, at: u64 },
    /// Taken back before the owner said yes.
    Cancel { id: String, at: u64 },
}

/// Where one request stands, as the person sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AskState {
    /// Not yet seen by the owner's Platter.
    Sending,
    /// The owner has it, waiting for a yes.
    Requested,
    /// Said yes; Lidarr is getting it.
    Getting,
    /// On the server.
    Ready,
    /// The owner said no this time.
    Declined,
    /// Nobody has a copy yet; it stays on the owner's list.
    NotFound,
    /// It can't be requested (why is in the note).
    Refused,
    /// It went wrong on the way.
    Failed,
    /// Taken back.
    Cancelled,
}

impl AskState {
    /// Nothing more will happen to it.
    pub fn settled(self) -> bool {
        matches!(self, AskState::Ready | AskState::Declined | AskState::Refused | AskState::Failed | AskState::Cancelled)
    }

    /// Still waiting on the owner or on Lidarr.
    pub fn open(self) -> bool {
        matches!(self, AskState::Sending | AskState::Requested | AskState::Getting | AskState::NotFound)
    }
}

/// From the owner's Platter to the person.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "camelCase")]
pub enum Down {
    /// Whether this person may ask, and whether their asks go straight
    /// through.
    #[serde(rename_all = "camelCase")]
    Policy { can_ask: bool, straight_away: bool, at: u64 },
    Status {
        id: String,
        state: AskState,
        #[serde(default)]
        progress: Option<u8>,
        #[serde(default)]
        note: Option<String>,
        at: u64,
    },
}

impl Down {
    pub fn at(&self) -> u64 {
        match self {
            Down::Policy { at, .. } | Down::Status { at, .. } => *at,
        }
    }
}

impl Up {
    pub fn at(&self) -> u64 {
        match self {
            Up::Hello { at } | Up::Ask { at, .. } | Up::Cancel { at, .. } => *at,
        }
    }
}

pub fn seal_up(p: &Pair, m: &Up) -> AppResult<String> {
    seal::seal_value(&p.up, m)
}

pub fn seal_down(p: &Pair, m: &Down) -> AppResult<String> {
    seal::seal_value(&p.down, m)
}

pub fn open_up(p: &Pair, text: &str) -> Result<Up, Rejected> {
    seal::open_value(&p.up, text)
}

pub fn open_down(p: &Pair, text: &str) -> Result<Down, Rejected> {
    seal::open_value(&p.down, text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(s: &str) -> Token {
        Token::new(s).unwrap()
    }

    fn item() -> AskItem {
        AskItem {
            release_group_id: Some("0f0e2a4c-1b2d-4c3e-9f00-000000000001".into()),
            title: "First Frost".into(),
            artist: "Cold Coast".into(),
            artist_mbid: None,
            album: None,
            song: false,
            isrc: None,
            thumb: None,
        }
    }

    #[test]
    fn the_same_token_and_server_make_the_same_channels_and_nothing_else_does() {
        let a = Pair::new(&tok("friend-token"), "machine-1");
        let b = Pair::new(&tok("friend-token"), "machine-1");
        assert_eq!(a.up.topic, b.up.topic);
        assert_ne!(a.up.topic, a.down.topic);
        assert_ne!(a.up.topic, Pair::new(&tok("friend-token"), "machine-2").up.topic);
        assert_ne!(a.up.topic, Pair::new(&tok("other-token"), "machine-1").up.topic);
        assert!(!a.up.topic.contains("friend"));
    }

    #[test]
    fn only_the_right_pair_and_direction_open_a_message() {
        let p = Pair::new(&tok("friend-token"), "m");
        let text = seal_up(&p, &Up::Ask { id: "a1".into(), item: item(), at: 5 }).unwrap();
        assert!(!text.contains("First Frost"));
        assert_eq!(open_up(&p, &text).unwrap(), Up::Ask { id: "a1".into(), item: item(), at: 5 });
        // Someone else's key, or the other direction, never opens it.
        assert!(open_up(&Pair::new(&tok("someone-else"), "m"), &text).is_err());
        assert!(open_down(&p, &text).is_err());
        let down = seal_down(&p, &Down::Policy { can_ask: true, straight_away: false, at: 6 }).unwrap();
        assert!(open_up(&p, &down).is_err());
        assert_eq!(open_down(&p, &down).unwrap().at(), 6);
    }

    #[test]
    fn items_must_be_sane() {
        assert!(item().sane());
        assert!(!AskItem { title: " ".into(), ..item() }.sane());
        assert!(!AskItem { title: "x".repeat(301), ..item() }.sane());
        assert!(!AskItem { thumb: Some("http://example.com/a.jpg".into()), ..item() }.sane());
        assert!(AskItem { thumb: Some("https://coverartarchive.org/a.jpg".into()), ..item() }.sane());
    }

    #[test]
    fn a_song_shows_its_album() {
        let s = AskItem { song: true, album: Some("Winter Songs".into()), ..item() };
        assert_eq!(s.album_title(), "Winter Songs");
        assert_eq!(item().album_title(), "First Frost");
    }
}
