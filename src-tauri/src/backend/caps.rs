//! What a music server can do. Features hide by these flags, never by the
//! kind of server: a server either has the capability or the feature that
//! needs it stays out of sight for its items.

use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

/// A set of capability flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Caps(u64);

macro_rules! caps {
    ($($(#[$doc:meta])* $name:ident = $bit:expr, $json:literal;)*) => {
        impl Caps {
            $($(#[$doc])* pub const $name: Caps = Caps(1 << $bit);)*

            /// Every flag with the name the UI knows it by.
            pub const NAMED: &'static [(&'static str, Caps)] = &[$(($json, Caps::$name)),*];
        }
    };
}

caps! {
    /// The server tells the app about library changes as they happen.
    LIVE_UPDATES = 0, "liveUpdates";
    /// The server can list what changed since a time.
    INCREMENTAL_SYNC = 1, "incrementalSync";
    /// Likes come with the time they were made.
    LIKE_DATES = 2, "likeDates";
    /// Star ratings besides likes.
    STAR_RATINGS = 3, "starRatings";
    /// A playlist row can be moved on the server.
    PLAYLIST_MOVE = 4, "playlistMove";
    /// A playlist cover can be uploaded.
    PLAYLIST_POSTER = 5, "playlistPoster";
    /// A playlist description can be set.
    PLAYLIST_SUMMARY = 6, "playlistSummary";
    /// Smart playlists can be saved on the server.
    SERVER_SMART_PLAYLISTS = 7, "serverSmartPlaylists";
    /// The server can convert a song to a lower bitrate.
    TRANSCODE = 8, "transcode";
    /// The server has lyrics for songs.
    LYRICS = 9, "lyrics";
    /// Its lyrics can be timed.
    SYNCED_LYRICS = 10, "syncedLyrics";
    /// The server makes radio from a song or artist.
    SERVER_RADIO = 11, "serverRadio";
    /// Sonic analysis data (similar sounding music).
    SONIC = 12, "sonic";
    /// Hand the queue to another player of the server's own.
    PLAY_QUEUE_HANDOFF = 13, "playQueueHandoff";
    /// The queue can be saved on the server and picked up elsewhere.
    RESUME_QUEUE = 14, "resumeQueue";
    /// Other players of the server can be controlled, and Platter can be one.
    REMOTE_PLAYERS = 15, "remotePlayers";
    /// What other people on the server are playing now.
    FRIEND_ACTIVITY = 16, "friendActivity";
    /// What other people on the server played before.
    FRIEND_HISTORY = 17, "friendHistory";
    /// Titles, artwork and grouping can be edited on the server.
    METADATA_EDIT = 18, "metadataEdit";
    /// Several people share one account and switch between them.
    HOME_USERS = 19, "homeUsers";
    /// The server says where each file is on its disks.
    REAL_FILE_PATHS = 20, "realFilePaths";
    /// The server can say whether a song's file can be read right now.
    CHECK_FILES = 21, "checkFiles";
    /// Loudness values for each song.
    LOUDNESS = 22, "loudness";
    /// Release types (album, single, EP, live) from the server.
    RELEASE_TYPES = 23, "releaseTypes";
    /// Play history with dates.
    SERVER_HISTORY = 24, "serverHistory";
    /// A play can be reported with the time it happened.
    BACKDATED_SCROBBLE = 25, "backdatedScrobble";
    /// Credits and file details for the song playing now.
    TRACK_DETAILS = 26, "trackDetails";
}

impl Caps {
    pub const NONE: Caps = Caps(0);

    /// Every flag there is.
    pub fn all() -> Caps {
        Caps::NAMED.iter().fold(Caps::NONE, |a, (_, c)| a | *c)
    }

    pub fn has(self, other: Caps) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn bits(self) -> u64 {
        self.0
    }

    /// The flag named `name` (as the UI spells it).
    pub fn named(name: &str) -> Option<Caps> {
        Caps::NAMED.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
    }
}

impl std::ops::BitOr for Caps {
    type Output = Caps;
    fn bitor(self, rhs: Caps) -> Caps {
        Caps(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Caps {
    fn bitor_assign(&mut self, rhs: Caps) {
        self.0 |= rhs.0;
    }
}

impl std::ops::Sub for Caps {
    type Output = Caps;
    fn sub(self, rhs: Caps) -> Caps {
        Caps(self.0 & !rhs.0)
    }
}

/// `{ "liveUpdates": true, ... }`, every flag present.
impl Serialize for Caps {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(Caps::NAMED.len()))?;
        for (name, c) in Caps::NAMED {
            m.serialize_entry(name, &self.has(*c))?;
        }
        m.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_distinct_and_named_once() {
        let mut seen = Caps::NONE;
        let mut names = std::collections::HashSet::new();
        for (name, c) in Caps::NAMED {
            assert_eq!(c.bits().count_ones(), 1, "{name}");
            assert!(!seen.has(*c), "{name} shares a bit");
            assert!(names.insert(*name), "{name} twice");
            seen |= *c;
            assert_eq!(Caps::named(name), Some(*c));
        }
        assert_eq!(seen, Caps::all());
        assert_eq!(Caps::named("nope"), None);
    }

    #[test]
    fn sets_combine_and_serialize_as_named_booleans() {
        let c = Caps::LYRICS | Caps::TRANSCODE;
        assert!(c.has(Caps::LYRICS));
        assert!(c.has(Caps::LYRICS | Caps::TRANSCODE));
        assert!(!c.has(Caps::LYRICS | Caps::SONIC));
        assert!(!(c - Caps::LYRICS).has(Caps::LYRICS));
        let v = serde_json::to_value(c).unwrap();
        assert_eq!(v["lyrics"], true);
        assert_eq!(v["sonic"], false);
        assert_eq!(v.as_object().unwrap().len(), Caps::NAMED.len());
    }
}
