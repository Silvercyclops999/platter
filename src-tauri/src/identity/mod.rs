//! Song identity: whether two songs, on one server or two, are the same.
//!
//! One place owns the answer for everything that needs it (other versions,
//! listening along, blends, imports, the Missing list):
//!
//! - [`fold`]    the folding rules for artist, title and version notes
//! - [`SongKey`] what a song is known by: ISRC, the recording id from file
//!   tags, else Plex's track id, and the folded names and length
//! - [`compare`] the ranked matcher: which tier matched, how sure it is,
//!   and whether it is the same recording or another version of the song
//! - [`index`]   a table of every synced song by folded artist and title,
//!   for fast lookups across servers
//!
//! Songs whose release status is unreleased (`db::release`) never match
//! anything. That rule is here and nowhere else.

pub mod fold;
pub mod index;

#[cfg(test)]
mod tests;

use crate::model::Track;
use crate::tags::FileTags;

/// Largest length difference for one song to be the same recording.
pub const SONG_SLACK_MS: u64 = 3_000;
/// The same, between songs of two albums already known to be the same
/// release (`db::merge` pairs songs of matching albums with this).
pub const ALBUM_SLACK_MS: u64 = 2_000;

/// What a song is known by. Ids are kept in one form (ISRC upper case,
/// MusicBrainz ids lower case); names are folded ([`fold`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SongKey {
    /// From the file's tags.
    pub isrc: Option<String>,
    /// MusicBrainz recording id from the file's tags.
    pub recording_mbid: Option<String>,
    /// MusicBrainz track id Plex matched. It names one release's track, so
    /// it only matches the same release.
    pub plex_track_mbid: Option<String>,
    /// The song's own artist (the album artist when the song has none).
    pub artist_fold: String,
    /// The album artist when it differs, a second name to match on (remix
    /// and collaboration albums often credit someone else).
    pub album_artist_fold: Option<String>,
    pub title_fold: String,
    /// Version notes split off the title ("demo", "live at wembley").
    pub version_fold: String,
    /// Album title, folded: breaks ties and decides generic titles.
    pub album_fold: String,
    /// 0 when unknown: names only, with less confidence.
    pub duration_ms: u64,
    /// Unreleased songs never match.
    pub unreleased: bool,
}

fn clean_id(v: Option<&str>) -> Option<String> {
    v.map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty())
}

impl SongKey {
    /// A song known only by names and length (an import row, a guest's song).
    pub fn from_names(artist: &str, title: &str, album: &str, duration_ms: u64) -> Self {
        let t = fold::fold_title(title);
        Self {
            artist_fold: fold::fold_artist(artist),
            title_fold: t.title,
            version_fold: t.version,
            album_fold: fold::fold_album(album),
            duration_ms,
            ..Default::default()
        }
    }

    /// A synced song plus what its file's tags said. `unreleased` comes
    /// from `db::release` for the song's album and file.
    pub fn from_track(t: &Track, tags: Option<&FileTags>, unreleased: bool) -> Self {
        let album_artist = fold::fold_artist(&t.artist_title);
        let own = t.track_artist.as_deref().map(fold::fold_artist).filter(|a| !a.is_empty());
        let (artist_fold, album_artist_fold) = match own {
            Some(a) if a != album_artist => (a, Some(album_artist).filter(|x| !x.is_empty())),
            Some(a) => (a, None),
            None => (album_artist, None),
        };
        let title = fold::fold_title(&t.title);
        Self {
            isrc: tags.and_then(|x| x.isrc.as_deref()).and_then(crate::tags::clean_isrc),
            recording_mbid: tags.and_then(|x| x.recording_mbid.as_deref()).and_then(crate::tags::clean_mbid),
            plex_track_mbid: clean_id(t.mbid.as_deref()),
            artist_fold,
            album_artist_fold,
            title_fold: title.title,
            version_fold: title.version,
            album_fold: fold::fold_album(&t.album_title),
            duration_ms: t.duration_ms,
            unreleased,
        }
    }

    fn artists(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.artist_fold.as_str())
            .chain(self.album_artist_fold.as_deref())
            .filter(|a| !a.is_empty())
    }
}

/// Which rule matched, strongest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    Isrc,
    RecordingMbid,
    PlexTrackMbid,
    /// Artist, title and version notes, with lengths within the slack.
    Text,
    /// Artist and title only: another version of the song.
    TextOtherVersion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatchKind {
    SameRecording,
    OtherVersion,
}

/// How two songs match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub tier: Tier,
    /// 0 to 100.
    pub confidence: u8,
    pub kind: MatchKind,
}

/// Where the songs are compared, which sets the length slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Context {
    /// Two songs on their own ([`SONG_SLACK_MS`]).
    #[default]
    Single,
    /// Songs of two albums already matched ([`ALBUM_SLACK_MS`]).
    MatchedAlbum,
}

impl Context {
    pub fn slack_ms(self) -> u64 {
        match self {
            Context::Single => SONG_SLACK_MS,
            Context::MatchedAlbum => ALBUM_SLACK_MS,
        }
    }
}

fn same_id(a: &Option<String>, b: &Option<String>) -> bool {
    matches!((a, b), (Some(x), Some(y)) if x == y)
}

/// Whether `a` and `b` are the same song, and how. None when they aren't,
/// or when either is unreleased.
///
/// Tiers in order: ISRC, MusicBrainz recording id, Plex's track id, then
/// the folded names. Names match when an artist name of each is the same
/// and the titles are; a bare generic title ("Intro") also needs the same
/// album. Matching names with the same version notes and lengths within
/// the slack are the same recording; a missing length (0) leaves the names
/// to decide, with less confidence. Other version notes, or lengths too far
/// apart (a live cut), make another version. Two different recording ids
/// are never the same recording.
pub fn compare(a: &SongKey, b: &SongKey, ctx: Context) -> Option<Match> {
    if a.unreleased || b.unreleased {
        return None;
    }
    let same = |tier, confidence| Some(Match { tier, confidence, kind: MatchKind::SameRecording });
    if same_id(&a.isrc, &b.isrc) {
        return same(Tier::Isrc, 99);
    }
    if same_id(&a.recording_mbid, &b.recording_mbid) {
        return same(Tier::RecordingMbid, 98);
    }
    if same_id(&a.plex_track_mbid, &b.plex_track_mbid) {
        return same(Tier::PlexTrackMbid, 95);
    }
    if a.title_fold.is_empty() || a.title_fold != b.title_fold {
        return None;
    }
    if !a.artists().any(|x| b.artists().any(|y| x == y)) {
        return None;
    }
    if fold::is_generic(&a.title_fold) && (a.album_fold.is_empty() || a.album_fold != b.album_fold) {
        return None;
    }
    let other = |confidence| Some(Match { tier: Tier::TextOtherVersion, confidence, kind: MatchKind::OtherVersion });
    let recordings_differ = matches!((&a.recording_mbid, &b.recording_mbid), (Some(x), Some(y)) if x != y);
    if fold::version_core(&a.version_fold) != fold::version_core(&b.version_fold) {
        return other(60);
    }
    if recordings_differ {
        return other(55);
    }
    if a.duration_ms == 0 || b.duration_ms == 0 {
        return same(Tier::Text, 70);
    }
    if a.duration_ms.abs_diff(b.duration_ms) <= ctx.slack_ms() {
        let exact_version = a.version_fold == b.version_fold;
        return same(Tier::Text, if exact_version { 90 } else { 85 });
    }
    other(50)
}
