//! The messages collaborative playlists and Blends send, inside sealed
//! packets ([`super::link`]). Metadata only: song identities (title,
//! artists, album, length, MusicBrainz ids, the song's id on a server both
//! people reach), names, roles and small taste summaries. Never audio, a
//! file path, a part key, a Plex token, a machine id or a server address.

use serde::{Deserialize, Serialize};

use crate::social::listen::wire::Song;

/// Someone as others see them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    /// A random id for this person in this playlist or Blend.
    pub id: String,
    pub name: String,
    /// A Plex profile picture as an `/ext/` art path (public, no token).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub av: Option<String>,
}

/// Who a song is. Everything here is metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Ident {
    /// Title and artist.
    pub n: String,
    pub a: String,
    /// Album title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub al: Option<String>,
    /// Length, ms.
    #[serde(default)]
    pub d: u64,
    /// MusicBrainz recording and release group ids.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rg: Option<String>,
    /// The song's id on the sender's server: `<server tag>:<rating key>`, the
    /// tag being the hash share links carry ([`crate::sid`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub k: Option<String>,
    /// Unreleased: never matched to a copy elsewhere.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub u: bool,
}

impl Ident {
    /// As listen along's matcher takes it.
    pub fn song(&self) -> Song {
        Song { mb: self.mb.clone(), n: Some(self.n.clone()), a: Some(self.a.clone()), d: self.d, u: self.u }
    }

    /// The same song whoever sends it: the MusicBrainz id, else artist and
    /// title folded.
    pub fn key(&self) -> String {
        match self.mb.as_deref().filter(|m| !m.is_empty()) {
            Some(m) => format!("mb:{}", m.to_ascii_lowercase()),
            None => format!("t:{}|{}", crate::db::merge::norm(&self.a), crate::db::merge::norm(&self.n)),
        }
    }
}

/// A playlist member's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Owner,
    /// Adds songs and removes the ones they added.
    Collaborator,
    /// Plays it; can't add.
    Listener,
    /// Left a public playlist: still sees it, isn't in "Shared with".
    Follower,
}

/// One song in a shared playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// The owner's row id (a Plex playlist item id).
    pub key: String,
    pub s: Ident,
    /// Who added it (a person id); None: the owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// When, unix seconds.
    #[serde(default)]
    pub at: i64,
}

/// A member in the owner's list, as everyone sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub p: Person,
    pub r: Role,
}

/// The playlist as the owner has it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u64,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sum: Option<String>,
    pub public: bool,
    pub owner: Person,
    /// Everyone else (followers left out).
    pub ppl: Vec<Member>,
    /// The receiver's role.
    pub you: Role,
    pub songs: Vec<Entry>,
}

/// Changes since version `from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta {
    pub from: u64,
    pub v: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add: Vec<Entry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub del: Vec<String>,
    /// The new order of every row, when it changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
}

/// A collaborator's change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum EditOp {
    Add { songs: Vec<Ident> },
    Remove { keys: Vec<String> },
}

/// A taste summary: what someone played most over recent weeks, with
/// weights, and their recent favourites. Built on their computer.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Taste {
    /// Top artists, heaviest first. Weight: share of plays, per mille.
    pub ar: Vec<WeightedArtist>,
    /// Top songs, heaviest first.
    pub so: Vec<WeightedSong>,
    /// Recent favourites (liked songs), newest first.
    #[serde(default)]
    pub fav: Vec<Ident>,
    /// When it was made, unix seconds.
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightedArtist {
    pub n: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mb: Option<String>,
    pub w: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WeightedSong {
    pub s: Ident,
    pub w: u32,
}

/// Why an invite was turned away.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Why {
    Used,
    Expired,
    Full,
    /// Gone: the playlist was deleted or the Blend ended.
    Gone,
}

/// What an invite is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Playlist,
    Blend,
}

/// The part of an answer only the person who redeemed the invite can open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub kind: Kind,
    /// The playlist's or Blend's id (random).
    pub g: String,
    /// Your id in it.
    pub id: String,
    /// The secret of your link with the owner (base64).
    pub link: String,
    pub owner: Person,
    pub title: String,
    /// A playlist: your role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// A Blend: the secret everyone in it shares (base64), and who's in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ppl: Vec<Person>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "camelCase")]
pub enum Msg {
    // ---- on an invite's channels
    /// Redeem an invite. `n` names this attempt (sent again unchanged while
    /// waiting); `pake` is the key agreement's first message.
    Join { n: String, pake: String, name: String, #[serde(default, skip_serializing_if = "Option::is_none")] av: Option<String> },
    Accepted { n: String, pake: String, sealed: String },
    Refused { n: String, why: Why },

    // ---- on a doorbell (people on the same server or Home)
    /// An invite for you: the link to open, who sent it and what it's for.
    Ring { link: String, from: String, kind: Kind, title: String },

    // ---- both ways on a link
    /// These packets arrived (every counter up to `upto`, and `also`).
    Ack { upto: u64, #[serde(default, skip_serializing_if = "Vec::is_empty")] also: Vec<u64> },

    // ---- owner → member
    Snapshot(Snapshot),
    Delta(Delta),
    /// A collaborator's edit was applied (or why not).
    Applied { n: String, ok: bool, #[serde(default, skip_serializing_if = "Option::is_none")] why: Option<String> },
    /// The owner deleted the playlist.
    Gone,
    /// You were taken off the playlist.
    Removed,

    // ---- member → owner
    /// Send me the playlist (joined, or out of step).
    Hello { name: String },
    Edit { n: String, op: EditOp },
    Leave,

    // ---- Blends, on the channel everyone in one shares
    Taste { p: Person, t: Taste },
    /// Someone new: send your taste again.
    Joined { p: Person },
    Left { id: String },
}
