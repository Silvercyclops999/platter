//! The messages inside sealed envelopes. Short field names keep a full state
//! (with up to [`super::UP_NEXT`] songs and every listener) under the relay's
//! 4 KB limit. Songs are the Plex server's rating keys; which server is
//! [`crate::social::listen::seal::server_hash`]. Nothing here is an address,
//! a port, a Plex token, a machine id or a stream URL.
//!
//! Songs can also carry who they are ([`Song`]: a MusicBrainz id, title,
//! artist, length), so a guest without the host's copy can play its own.
//! Those fields are optional both ways: a guest from before them follows by
//! rating key, and a host from before them sends none (its guests then play
//! only the host's own copies).

use serde::{Deserialize, Serialize};

/// A participant as others see them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub id: String,
    pub name: String,
    /// Optional picture; none is sent today (others see the name's letter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub av: Option<String>,
}

/// Who a song is, for a guest looking for its own copy. Metadata only.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Song {
    /// The MusicBrainz track id, when Plex matched the song.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mb: Option<String>,
    /// Title and artist. Left out of an unreleased song while the host hides
    /// those from guests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a: Option<String>,
    /// Length, ms.
    #[serde(default)]
    pub d: u64,
    /// Unreleased: never matched to a copy on another server.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub u: bool,
}

/// One song in the shared queue and who added it (a person id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queued {
    pub k: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Who the song is (dropped from the end of a long queue to fit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub s: Option<Song>,
}

/// What guests follow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// The Plex server the songs are on (hashed per session).
    pub srv: String,
    /// The song playing (a rating key).
    pub t: Option<String>,
    /// Who the song playing is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ts: Option<Song>,
    /// Who added the song playing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tby: Option<String>,
    /// Position and length, ms.
    pub p: u64,
    pub d: u64,
    pub pl: bool,
    /// The songs after this one, in play order.
    pub q: Vec<Queued>,
    /// Guests may change what's playing (else they may only add).
    pub ctl: bool,
    /// Everyone in the session, host first.
    pub ppl: Vec<Person>,
}

/// What a guest asks the host to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Op {
    Play,
    Pause,
    Next,
    Previous,
    Seek { ms: u64 },
    /// `songs`: who each id is in the guest's own library, in order. The
    /// host adds only ids it describes the same way, so a guest can add
    /// songs it can play but can't learn what other ids are.
    Add {
        ids: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        songs: Vec<Song>,
    },
    /// Move the song at `from` in the up next list to `to`.
    Move { from: usize, to: usize },
}

/// Why the host turned a join away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Why {
    Used,
    Expired,
    Full,
}

/// The part of a welcome only the guest who answered the invite can open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    /// The guest's id in the session.
    pub id: String,
    /// The guest's own secret (base64): its channels with the host.
    pub g: String,
    /// The broadcast secret (base64) and its epoch.
    pub b: String,
    pub e: u32,
    pub host: Person,
    pub srv: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "camelCase")]
pub enum Msg {
    // ---- on an invite's channels
    /// Guest → host: redeem the invite. `n` names this attempt (sent again
    /// unchanged if the answer is lost); `pake` is the key agreement's first
    /// message (base64).
    Join { n: String, pake: String, name: String },
    /// Host → guest: the agreement's answer, and [`Welcome`] sealed with the
    /// agreed key.
    Accepted { n: String, pake: String, sealed: String },
    Refused { n: String, why: Why },

    // ---- host → guests
    State(State),
    /// The host ended the session.
    Ended,
    /// Host → one guest: a new broadcast secret (someone left or was removed).
    Rekey { b: String, e: u32 },
    /// Host → one guest: you were removed.
    Removed,
    /// Host → one guest: a request was turned down.
    Denied { msg: String },

    // ---- guest → host
    /// Send me the state (after joining, a reconnect or a rekey).
    Hello { name: String },
    /// Still here.
    Ping,
    Cmd(Op),
    Leave,
    /// The song playing (`t`, the host's id) has no copy here; None: this
    /// guest hears the session again. A host from before this drops it.
    CantHear {
        #[serde(default)]
        t: Option<String>,
    },

    // ---- "Let them listen from my server" (a peer from before these drops them)
    /// Host → one guest: may I share my Plex server's music with you?
    Offer,
    /// Guest → host: my Plex username for the invite. None: I don't take
    /// invites inside Platter (the host shares from Settings instead).
    PlexUser {
        #[serde(default)]
        u: Option<String>,
    },
    /// Host → one guest: the invite went out (or you already have access);
    /// look for it.
    Shared,
}
