//! "Hey Siri, play … on Platter": what Siri understood, matched against the
//! library copy on the phone, so it works with no signal for what's there.
//! The iOS side (the intent handling) hands the request to [`resolve`] and
//! plays what comes back; likes and "add this to my playlist" find their
//! song and playlist with [`song_for`] and [`playlist_named`].

use std::collections::HashSet;

use rusqlite::Connection;

use super::{CarAction, Kept};
use crate::audio::queue::Repeat;
use crate::audio::source::{ContextKind, PlayContext};
use crate::db::DbHandle;
use crate::db::types::SearchFilter;
use crate::error::AppResult;

/// What Siri says the request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SiriKind {
    Song,
    Album,
    Artist,
    Playlist,
    Genre,
    /// A station or radio ("play Kanye West radio").
    Station,
    /// Siri didn't say ("play Watch the Throne on Platter").
    #[default]
    Unknown,
}

impl SiriKind {
    /// From Intents' `INMediaItemType`: song 1, album 2, artist 3, genre 4,
    /// playlist 5, and the stations (music 9, radio 16, station 17,
    /// algorithmic 19); anything else ("music") is left to the search.
    pub fn from_media_type(t: isize) -> Self {
        match t {
            1 => SiriKind::Song,
            2 => SiriKind::Album,
            3 => SiriKind::Artist,
            4 => SiriKind::Genre,
            5 => SiriKind::Playlist,
            9 | 16 | 17 | 19 => SiriKind::Station,
            _ => SiriKind::Unknown,
        }
    }
}

/// Where in the queue it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SiriQueue {
    /// Play it now.
    #[default]
    Now,
    /// "Play this next".
    Next,
    /// "Add this to the queue".
    Later,
}

impl SiriQueue {
    /// From `INPlaybackQueueLocation`: next 2, later 3; now and unknown play now.
    pub fn from_location(t: isize) -> Self {
        match t {
            2 => SiriQueue::Next,
            3 => SiriQueue::Later,
            _ => SiriQueue::Now,
        }
    }
}

/// Which of several Siri wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SiriSort {
    #[default]
    Any,
    /// "The new Drake album".
    Newest,
    /// "Kanye's popular songs", "trending".
    Popular,
}

impl SiriSort {
    /// From `INMediaSortOrder`: newest 1, popular 5, trending 7.
    pub fn from_order(t: isize) -> Self {
        match t {
            1 => SiriSort::Newest,
            5 | 7 => SiriSort::Popular,
            _ => SiriSort::Any,
        }
    }
}

/// From `INPlaybackRepeatMode`: none 1, all 2, one 3; unknown leaves repeat
/// as it is.
pub fn repeat_from(t: isize) -> Option<Repeat> {
    match t {
        1 => Some(Repeat::Off),
        2 => Some(Repeat::All),
        3 => Some(Repeat::One),
        _ => None,
    }
}

/// A request as Siri understood it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SiriAsk {
    /// The item's name ("Watch the Throne", "Liked Songs"); None for "play
    /// some music on Platter".
    pub name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub kind: SiriKind,
    pub shuffle: bool,
    pub repeat: Option<Repeat>,
    pub queue: SiriQueue,
    /// "Resume", "play Platter": carry on with what was playing.
    pub resume: bool,
    pub genres: Vec<String>,
    pub moods: Vec<String>,
    pub sort: SiriSort,
    /// About the playing song ("play more like this", "I like this song").
    pub this: bool,
}

impl SiriAsk {
    fn name(&self) -> Option<&str> {
        self.name.as_deref().map(str::trim).filter(|n| !n.is_empty())
    }

    fn artist(&self) -> Option<&str> {
        self.artist.as_deref().map(str::trim).filter(|n| !n.is_empty())
    }
}

/// What to play for it.
#[derive(Debug, Clone, PartialEq)]
pub struct SiriPlay {
    pub context: PlayContext,
    /// Start here (a song asked for by name, played within its album).
    pub start_track_id: Option<String>,
    pub shuffle: bool,
    /// Repeat to set, when Siri heard one.
    pub repeat: Option<Repeat>,
    /// Now, next, or at the end of the queue.
    pub queue: SiriQueue,
    /// What it is, for Siri to say back.
    pub title: String,
}

/// The answer to a play request.
#[derive(Debug, Clone, PartialEq)]
pub enum SiriAnswer {
    Play(SiriPlay),
    /// Carry on with what's in the player.
    Resume,
}

/// The song playing now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Playing {
    pub track_id: String,
    pub title: String,
}

/// What the request is heard against besides the library.
#[derive(Debug, Clone, Default)]
pub struct Situation {
    pub playing: Option<Playing>,
    /// Something is paused or stopped in the player and can carry on.
    pub can_resume: bool,
    /// No signal: only what's on the phone plays.
    pub offline: Option<Kept>,
}

fn words(s: &str) -> String {
    s.trim().to_lowercase().replace(['\u{2019}', '\''], "")
}

fn is_liked(name: &str) -> bool {
    matches!(
        words(name).as_str(),
        "liked songs" | "my liked songs" | "liked" | "likes" | "my likes" | "favorites" | "favourites" | "my favorites"
            | "my favourites" | "loved songs"
    )
}

fn is_everything(name: &str) -> bool {
    matches!(
        words(name).as_str(),
        "my library" | "library" | "my music" | "music" | "everything" | "all my music" | "all songs" | "my songs"
    )
}

fn context(kind: ContextKind, id: &str) -> PlayContext {
    PlayContext { kind, id: Some(id.to_string()), track_ids: None }
}

fn play(context: PlayContext, title: impl Into<String>) -> SiriPlay {
    SiriPlay { context, start_track_id: None, shuffle: false, repeat: None, queue: SiriQueue::Now, title: title.into() }
}

/// Matches the request against the library; None when nothing fits (or,
/// with no signal, nothing that fits is on the phone). `shuffle_all` makes
/// the "shuffle everything" context (a random draw, as CarPlay's Shuffle my
/// library does; with no signal, of the downloaded songs).
pub fn resolve(
    db: &DbHandle,
    ask: &SiriAsk,
    at: &Situation,
    shuffle_all: impl FnOnce() -> AppResult<PlayContext>,
) -> AppResult<Option<SiriAnswer>> {
    let picked = match pick(db, ask, at)? {
        Some(Pick::Resume) => return Ok(Some(SiriAnswer::Resume)),
        Some(Pick::Everything) => {
            let mut p = play(shuffle_all()?, "your library");
            p.shuffle = true;
            Some(p)
        }
        Some(Pick::Play(p)) => Some(p),
        None => None,
    };
    let Some(mut p) = picked else { return Ok(None) };
    if !p.shuffle {
        p.shuffle = ask.shuffle;
    }
    // "Play Runaway on repeat": that song over and over.
    p.repeat = match (ask.repeat, &p.start_track_id) {
        (Some(Repeat::All), Some(_)) if ask.kind == SiriKind::Song => Some(Repeat::One),
        (r, _) => r,
    };
    p.queue = ask.queue;
    if let Some(kept) = &at.offline {
        let here = match &p.start_track_id {
            Some(t) => kept.tracks.contains(t),
            None => kept.plays(&CarAction::Play(p.context.clone())),
        };
        if !here {
            return Ok(None);
        }
    }
    Ok(Some(SiriAnswer::Play(p)))
}

enum Pick {
    Play(SiriPlay),
    Everything,
    Resume,
}

fn pick(db: &DbHandle, ask: &SiriAsk, at: &Situation) -> AppResult<Option<Pick>> {
    let name = ask.name();
    let tags: Vec<&str> = ask.genres.iter().chain(&ask.moods).map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    // "Play more like this": radio from the playing song.
    if (ask.this || (ask.kind == SiriKind::Station && name.is_none() && ask.artist().is_none()))
        && let Some(p) = &at.playing
    {
        let ctx = context(ContextKind::Radio, &format!("track:{}", p.track_id));
        return Ok(Some(Pick::Play(play(ctx, format!("music like {}", p.title)))));
    }
    // "Play Kanye West radio", "play music like Kanye West".
    if ask.kind == SiriKind::Station
        && let Some(a) = find_artist(db, name.or(ask.artist()).unwrap_or_default())?
    {
        let ctx = context(ContextKind::Radio, &format!("artist:{}", a.id));
        return Ok(Some(Pick::Play(play(ctx, format!("music like {}", a.title)))));
    }
    if ask.kind == SiriKind::Genre {
        let mut all = tags.clone();
        all.extend(name);
        return Ok(mix_for_tags(db, &all)?.map(Pick::Play));
    }
    let Some(name) = name else {
        if let Some(artist) = ask.artist() {
            // "Play the new Drake album", "play something by X".
            return Ok(by_artist(db, artist, ask.sort)?.map(Pick::Play));
        }
        if !tags.is_empty() {
            // "Play hip-hop", "play something chill".
            return Ok(mix_for_tags(db, &tags)?.map(Pick::Play));
        }
        // "Play Platter", "play something": carry on, else the first mix,
        // else everything shuffled.
        if (ask.resume || at.can_resume) && !ask.shuffle {
            return Ok(Some(Pick::Resume));
        }
        if !ask.shuffle
            && at.offline.is_none()
            && let Some(m) = mixes(db)?.into_iter().next()
        {
            return Ok(Some(Pick::Play(play(context(ContextKind::Mix, &m.id), m.title))));
        }
        return Ok(Some(Pick::Everything));
    };
    if is_liked(name) {
        return Ok(Some(Pick::Play(play(context(ContextKind::Liked, "liked"), "your liked songs"))));
    }
    if is_everything(name) && ask.kind != SiriKind::Song {
        return Ok(Some(Pick::Everything));
    }
    // A mix by its name ("play Daily Mix 2", "play my daylist").
    if matches!(ask.kind, SiriKind::Playlist | SiriKind::Unknown) {
        let n = words(name);
        let n = n.strip_prefix("my ").unwrap_or(&n);
        if let Some(m) = mixes(db)?.into_iter().find(|m| words(&m.title) == n) {
            return Ok(Some(Pick::Play(play(context(ContextKind::Mix, &m.id), m.title))));
        }
    }
    // "Play Kanye's newest album": Siri may hear the artist as the name.
    if ask.sort == SiriSort::Newest
        && matches!(ask.kind, SiriKind::Album | SiriKind::Artist | SiriKind::Unknown)
        && let Some(p) = by_artist(db, ask.artist().unwrap_or(name), SiriSort::Newest)?
    {
        return Ok(Some(Pick::Play(p)));
    }
    // The name, plus the artist when Siri heard one, is a library search.
    let query = match ask.artist() {
        Some(a) => format!("{name} {a}"),
        None => name.to_string(),
    };
    let filter = match ask.kind {
        SiriKind::Song => SearchFilter::Songs,
        SiriKind::Album => SearchFilter::Albums,
        SiriKind::Artist => SearchFilter::Artists,
        SiriKind::Playlist => SearchFilter::Playlists,
        _ => SearchFilter::All,
    };
    let found = crate::db::search::search(db, &query, filter, Some(5))?;
    let pick = match ask.kind {
        SiriKind::Song => found.songs.first().map(song),
        SiriKind::Album => found.albums.first().map(album),
        SiriKind::Artist => found.artists.first().map(artist),
        SiriKind::Playlist => found.playlists.first().map(playlist),
        _ => {
            use crate::db::types::SearchKind;
            match found.top {
                Some(top) => match top.kind {
                    SearchKind::Album => top.album.as_ref().map(album),
                    SearchKind::Artist => top.artist.as_ref().map(artist),
                    SearchKind::Playlist => top.playlist.as_ref().map(playlist),
                    _ => top.track.as_ref().map(song),
                },
                None => None,
            }
        }
    };
    // A match found only through the artist's name isn't the thing asked
    // for: it has to share a word with the name.
    let pick = pick.filter(|p| shares_a_word(&p.matched, name));
    if let Some(p) = pick {
        return Ok(Some(Pick::Play(p.play)));
    }
    // Nothing by that name, but the artist is known: play the artist.
    match ask.artist() {
        Some(a) => Ok(by_artist(db, a, SiriSort::Any)?.map(Pick::Play)),
        None => Ok(None),
    }
}

fn find_artist(db: &DbHandle, name: &str) -> AppResult<Option<crate::model::Artist>> {
    if name.trim().is_empty() {
        return Ok(None);
    }
    let found = crate::db::search::search(db, name, SearchFilter::Artists, Some(3))?;
    Ok(found.artists.into_iter().next().filter(|a| shares_a_word(&a.title, name)))
}

/// The artist, or with [`SiriSort::Newest`] their newest album.
fn by_artist(db: &DbHandle, artist_name: &str, sort: SiriSort) -> AppResult<Option<SiriPlay>> {
    let Some(a) = find_artist(db, artist_name)? else { return Ok(None) };
    if sort == SiriSort::Newest
        && let Some(newest) = db.read(|c| crate::db::queries::artist_albums(c, &a.id, 1))?.into_iter().next()
    {
        return Ok(Some(album(&newest).play));
    }
    Ok(Some(artist(&a).play))
}

/// A stored mix, as Siri matches it.
#[derive(Debug, Clone, PartialEq)]
struct SiriMix {
    id: String,
    title: String,
    tag: Option<String>,
}

/// The stored mixes, in their shelf order.
fn mixes(db: &DbHandle) -> AppResult<Vec<SiriMix>> {
    db.read(|c: &Connection| {
        let mut st = c.prepare_cached("SELECT id, title, tag FROM mixes ORDER BY position")?;
        let rows = st.query_map([], |r| Ok(SiriMix { id: r.get(0)?, title: r.get(1)?, tag: r.get(2)? }))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
}

/// The genre or mood mix for any of `tags` ("hip-hop", "chill").
fn mix_for_tags(db: &DbHandle, tags: &[&str]) -> AppResult<Option<SiriPlay>> {
    let wanted: HashSet<String> = tags.iter().map(|t| bare(t)).filter(|t| !t.is_empty()).collect();
    if wanted.is_empty() {
        return Ok(None);
    }
    Ok(mixes(db)?
        .into_iter()
        .find(|m| m.tag.as_deref().is_some_and(|t| wanted.contains(&bare(t))) || wanted.contains(&bare(&m.title)))
        .map(|m| play(context(ContextKind::Mix, &m.id), m.title)))
}

/// Letters and digits only: "Hip-Hop", "hip hop" and "hiphop" are one tag.
fn bare(s: &str) -> String {
    words(s).chars().filter(|c| c.is_alphanumeric()).collect()
}

/// A pick and the title it was matched on.
struct Picked {
    play: SiriPlay,
    matched: String,
}

/// Words too common to tell two titles apart.
const STOP_WORDS: &[&str] = &["the", "a", "an", "of", "and", "in", "on", "to", "my", "by", "for", "it", "is"];

fn split(s: &str) -> Vec<String> {
    words(s).split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect()
}

/// The words of `name` that tell titles apart.
fn telling(name: &str) -> Vec<String> {
    split(name).into_iter().filter(|w| w.len() > 1 && !STOP_WORDS.contains(&w.as_str())).collect()
}

fn shares_a_word(title: &str, name: &str) -> bool {
    let title = split(title);
    telling(name).iter().any(|w| title.contains(w))
}

/// Every telling word of `name` is in `title` (or, when it has none, the
/// two are the same words).
fn has_all_words(title: &str, name: &str) -> bool {
    let t = split(title);
    let want = telling(name);
    if want.is_empty() {
        return !t.is_empty() && t == split(name);
    }
    want.iter().all(|w| t.contains(w))
}

fn song(t: &crate::model::Track) -> Picked {
    Picked {
        matched: t.title.clone(),
        play: SiriPlay {
            start_track_id: Some(t.id.clone()),
            ..play(
                context(ContextKind::Album, &t.album_id),
                format!("{} by {}", t.title, t.track_artist.clone().unwrap_or_else(|| t.artist_title.clone())),
            )
        },
    }
}

fn album(a: &crate::model::Album) -> Picked {
    Picked { matched: a.title.clone(), play: play(context(ContextKind::Album, &a.id), format!("{} by {}", a.title, a.artist_title)) }
}

fn artist(a: &crate::model::Artist) -> Picked {
    Picked { matched: a.title.clone(), play: play(context(ContextKind::Artist, &a.id), a.title.clone()) }
}

fn playlist(p: &crate::model::Playlist) -> Picked {
    Picked { matched: p.title.clone(), play: play(context(ContextKind::Playlist, &p.id), p.title.clone()) }
}

/// The song a like, dislike or "add this to my playlist" is about: the one
/// Siri named, else the one playing. The id and how Siri says it.
pub fn song_for(db: &DbHandle, ask: &SiriAsk, playing: Option<&Playing>) -> AppResult<Option<(String, String)>> {
    let named = ask.name().filter(|_| !ask.this && matches!(ask.kind, SiriKind::Song | SiriKind::Unknown));
    let Some(name) = named else {
        return Ok(playing.map(|p| (p.track_id.clone(), p.title.clone())));
    };
    let query = match ask.artist() {
        Some(a) => format!("{name} {a}"),
        None => name.to_string(),
    };
    let found = crate::db::search::search(db, &query, SearchFilter::Songs, Some(5))?;
    Ok(found.songs.into_iter().find(|t| has_all_words(&t.title, name)).map(|t| (t.id, t.title)))
}

/// The playlist called `name` ("Road Trip", "my road trip playlist").
pub fn playlist_named(db: &DbHandle, name: &str) -> AppResult<Option<(String, String)>> {
    let n = words(name);
    let n = n.strip_prefix("my ").unwrap_or(&n);
    let n = n.strip_suffix(" playlist").unwrap_or(n).trim().to_string();
    if n.is_empty() {
        return Ok(None);
    }
    let found = crate::db::search::search(db, &n, SearchFilter::Playlists, Some(5))?;
    Ok(found.playlists.into_iter().find(|p| has_all_words(&p.title, &n)).map(|p| (p.id, p.title)))
}

/// What to look for among music you don't have when the library has nothing
/// ("play Middle Child on Platter"): the name, and the artist when Siri
/// heard one. None for playlists, genres, stations and the library's own
/// names.
pub fn catalogue_query(ask: &SiriAsk) -> Option<String> {
    if !matches!(ask.kind, SiriKind::Song | SiriKind::Album | SiriKind::Artist | SiriKind::Unknown) || ask.this {
        return None;
    }
    let Some(name) = ask.name() else {
        // "Play something by X" with X not in the library.
        return ask.artist().map(str::to_string);
    };
    if is_liked(name) || is_everything(name) {
        return None;
    }
    Some(match ask.artist() {
        Some(a) => format!("{name} {a}"),
        None => name.to_string(),
    })
}

/// The first catalogue song that is the one asked for, as a tap: for a song
/// its title has every telling word of the name; for an album its album
/// does; for an artist (no name) it's theirs, and the first is their
/// best-known. When Siri heard an artist, the song is by them.
pub fn tap_for(ask: &SiriAsk, songs: &[crate::discover::CatSong]) -> Option<crate::lidarr::tap::TapInput> {
    let (by, name) = match (ask.kind, ask.name()) {
        (SiriKind::Artist, Some(n)) => (Some(ask.artist().unwrap_or(n)), None),
        (_, None) => (ask.artist(), None),
        (_, Some(n)) => (ask.artist(), Some(n)),
    };
    by.or(name)?;
    let s = songs.iter().find(|s| {
        let named = match (ask.kind, name) {
            (_, None) => true,
            (SiriKind::Album, Some(n)) => s.album.as_deref().is_some_and(|a| has_all_words(a, n)),
            (_, Some(n)) => has_all_words(&s.title, n),
        };
        named && by.is_none_or(|a| has_all_words(&s.artist, a))
    })?;
    Some(crate::lidarr::tap::TapInput {
        title: s.title.clone(),
        artist: s.artist.clone(),
        artist_mbid: s.artist_mbid.clone(),
        album: s.album.clone(),
        release_group_id: s.release_group_id.clone(),
        duration_ms: s.duration_ms,
        thumb: s.thumb.clone(),
        ..Default::default()
    })
}

/// How getting a song for Siri went within the time Siri waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Getting {
    /// It's on its way (or, for someone the server is shared with, asked for).
    Started,
    /// Nothing among music you don't have fits.
    NothingFits,
    Failed,
    /// Siri's wait ran out first; the get carries on.
    StillGoing,
}

/// Runs `get` on its own task and waits up to `wait` for how it went. The
/// wait running out doesn't stop the get.
pub async fn get_within<F>(get: F, wait: std::time::Duration) -> Getting
where
    F: std::future::Future<Output = AppResult<bool>> + Send + 'static,
{
    let task = tokio::spawn(get);
    match tokio::time::timeout(wait, task).await {
        Ok(Ok(Ok(true))) => Getting::Started,
        Ok(Ok(Ok(false))) => Getting::NothingFits,
        Ok(Ok(Err(e))) => {
            tracing::warn!("Siri: the song couldn't be got: {e}");
            Getting::Failed
        }
        Ok(Err(e)) => {
            tracing::warn!("Siri: the get stopped: {e}");
            Getting::Failed
        }
        Err(_) => {
            tracing::info!("Siri: still getting the song after Siri's wait");
            Getting::StillGoing
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::db::fake::{self, FakeLibrary, FakeSource};
    use crate::db::source::LibrarySource;

    fn asked(name: &str, artist: Option<&str>, kind: SiriKind) -> SiriAsk {
        SiriAsk { name: Some(name.into()), artist: artist.map(Into::into), kind, ..Default::default() }
    }

    fn cat(title: &str, artist: &str) -> crate::discover::CatSong {
        crate::discover::CatSong {
            recording_id: format!("r-{title}"),
            title: title.into(),
            artist: artist.into(),
            artist_mbid: None,
            album: Some(format!("{title} album")),
            release_group_id: Some(format!("rg-{title}")),
            duration_ms: Some(200_000),
            thumb: None,
        }
    }

    #[test]
    fn a_song_the_library_lacks_is_looked_for_and_tapped() {
        assert_eq!(catalogue_query(&asked("Middle Child", None, SiriKind::Song)).as_deref(), Some("Middle Child"));
        assert_eq!(catalogue_query(&asked("Middle Child", Some("J. Cole"), SiriKind::Unknown)).as_deref(), Some("Middle Child J. Cole"));
        assert_eq!(catalogue_query(&asked("my liked songs", None, SiriKind::Song)), None);
        assert_eq!(catalogue_query(&asked("Road Trip", None, SiriKind::Playlist)), None);
        assert_eq!(catalogue_query(&asked("hip hop", None, SiriKind::Genre)), None);
        let songs = [cat("Childhood", "Someone Else"), cat("Middle Child", "J. Cole")];
        let t = tap_for(&asked("Middle Child", Some("J. Cole"), SiriKind::Song), &songs).unwrap();
        assert_eq!((t.title.as_str(), t.artist.as_str(), t.release_group_id.as_deref()), ("Middle Child", "J. Cole", Some("rg-Middle Child")));
        // Without an artist the first match wins; a wrong artist or name finds nothing.
        assert_eq!(tap_for(&asked("Middle Child", None, SiriKind::Song), &songs[1..]).unwrap().artist, "J. Cole");
        assert!(tap_for(&asked("Middle Child", Some("Drake"), SiriKind::Song), &songs).is_none());
        assert!(tap_for(&asked("Wet Dreamz", None, SiriKind::Song), &songs).is_none());
    }

    #[test]
    fn a_catalogue_tap_needs_every_word_and_the_whole_artist() {
        // One shared word isn't the song asked for.
        let songs = [cat("Middle Child", "J. Cole")];
        assert!(tap_for(&asked("Child of Mine", None, SiriKind::Unknown), &songs).is_none());
        // "Ye" is not "Kanye": artists match by whole words.
        let songs = [cat("Runaway", "Kanye West")];
        assert!(tap_for(&asked("Runaway", Some("Ye"), SiriKind::Song), &songs).is_none());
        assert!(tap_for(&asked("Runaway", Some("kanye"), SiriKind::Song), &songs).is_some());
    }

    #[test]
    fn albums_and_artists_you_dont_have_are_got_through_a_song() {
        let mut donda = cat("Jail", "Kanye West");
        donda.album = Some("Donda".into());
        let songs = [cat("Runaway", "Kanye West"), donda];
        assert_eq!(catalogue_query(&asked("Donda", Some("Kanye West"), SiriKind::Album)).as_deref(), Some("Donda Kanye West"));
        let t = tap_for(&asked("Donda", None, SiriKind::Album), &songs).unwrap();
        assert_eq!((t.title.as_str(), t.album.as_deref()), ("Jail", Some("Donda")));
        // An artist: their first (best-known) song, which gets its album.
        let t = tap_for(&asked("Kanye West", None, SiriKind::Artist), &songs).unwrap();
        assert_eq!(t.title, "Runaway");
        let by = SiriAsk { artist: Some("Kanye West".into()), ..Default::default() };
        assert_eq!(catalogue_query(&by).as_deref(), Some("Kanye West"));
        assert_eq!(tap_for(&by, &songs).unwrap().title, "Runaway");
        assert!(tap_for(&asked("Drake", None, SiriKind::Artist), &songs).is_none());
    }

    async fn library() -> (DbHandle, tempfile::TempDir) {
        let mut lib = FakeLibrary { sections: fake::generated(0, 0, 0).sections, ..Default::default() };
        let ye = fake::artist("10", "Kanye West");
        let tame = fake::artist("11", "Tame Impala");
        let wtt = fake::album("20", "Watch the Throne", &ye, 2011, Some("album"), &[]);
        let curr = fake::album("21", "Currents", &tame, 2015, Some("album"), &[]);
        let donda = fake::album("22", "Donda", &ye, 2021, Some("album"), &[]);
        lib.tracks.push(fake::track("30", "Otis", &wtt, 1, 1));
        lib.tracks.push(fake::track("31", "Niggas in Paris", &wtt, 1, 2));
        lib.tracks.push(fake::track("32", "Let It Happen", &curr, 1, 1));
        lib.tracks.push(fake::track("33", "The Less I Know the Better", &curr, 1, 2));
        lib.tracks.push(fake::track("34", "Jail", &donda, 1, 1));
        lib.albums = vec![wtt, curr, donda];
        lib.artists = vec![ye, tame];
        lib.playlists.push((fake::playlist("40", "Road Trip", 2), vec!["30".into(), "32".into()]));
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("library.db")).unwrap();
        let src = Arc::new(FakeSource::new(lib));
        db.sync_once(src as Arc<dyn LibrarySource>, true).await.unwrap();
        db.write(|c| {
            for (i, (id, kind, title, tag)) in [
                ("m1", "daily", "Daily Mix 1", None),
                ("m2", "daily", "Daily Mix 2", None),
                ("m3", "genre", "Hip-Hop Mix", Some("Hip-Hop")),
                ("m4", "mood", "Chill Mix", Some("Chill")),
            ]
            .into_iter()
            .enumerate()
            {
                c.execute(
                    "INSERT INTO mixes (id, kind, position, title, description, color, tag, day, generated_at)
                     VALUES (?1, ?2, ?3, ?4, '', '#000000', ?5, '2026-10-21', 0)",
                    rusqlite::params![id, kind, i as i64, title, tag],
                )?;
            }
            Ok(())
        })
        .unwrap();
        (db, dir)
    }

    fn all() -> AppResult<PlayContext> {
        Ok(PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec!["30".into()]) })
    }

    fn ask(name: &str, kind: SiriKind) -> SiriAsk {
        SiriAsk { name: Some(name.into()), kind, ..Default::default() }
    }

    fn answer(db: &DbHandle, a: &SiriAsk, at: &Situation) -> Option<SiriAnswer> {
        resolve(db, a, at, all).unwrap()
    }

    fn played(db: &DbHandle, a: &SiriAsk) -> SiriPlay {
        played_at(db, a, &Situation::default())
    }

    fn played_at(db: &DbHandle, a: &SiriAsk, at: &Situation) -> SiriPlay {
        match answer(db, a, at) {
            Some(SiriAnswer::Play(p)) => p,
            other => panic!("expected something to play for {a:?}, got {other:?}"),
        }
    }

    fn playing() -> Situation {
        Situation { playing: Some(Playing { track_id: "32".into(), title: "Let It Happen".into() }), ..Default::default() }
    }

    #[test]
    fn common_words_dont_make_a_match() {
        assert!(!shares_a_word("The Less I Know the Better", "The Wall"));
        assert!(shares_a_word("The Less I Know the Better", "less i know"));
        assert!(shares_a_word("Otis", "otis"));
        assert!(has_all_words("The Less I Know the Better", "less i know better"));
        assert!(!has_all_words("Middle Child", "child of mine"));
        assert!(has_all_words("It", "it"), "a title of only common words matches itself");
    }

    #[test]
    fn siri_values_map() {
        assert_eq!(SiriKind::from_media_type(1), SiriKind::Song);
        assert_eq!(SiriKind::from_media_type(2), SiriKind::Album);
        assert_eq!(SiriKind::from_media_type(3), SiriKind::Artist);
        assert_eq!(SiriKind::from_media_type(4), SiriKind::Genre);
        assert_eq!(SiriKind::from_media_type(5), SiriKind::Playlist);
        assert_eq!(SiriKind::from_media_type(17), SiriKind::Station);
        assert_eq!(SiriKind::from_media_type(0), SiriKind::Unknown);
        assert_eq!(SiriKind::from_media_type(18), SiriKind::Unknown);
        assert_eq!(SiriQueue::from_location(2), SiriQueue::Next);
        assert_eq!(SiriQueue::from_location(3), SiriQueue::Later);
        assert_eq!(SiriQueue::from_location(1), SiriQueue::Now);
        assert_eq!(SiriSort::from_order(1), SiriSort::Newest);
        assert_eq!(SiriSort::from_order(5), SiriSort::Popular);
        assert_eq!(SiriSort::from_order(3), SiriSort::Any);
        assert_eq!(repeat_from(1), Some(Repeat::Off));
        assert_eq!(repeat_from(2), Some(Repeat::All));
        assert_eq!(repeat_from(3), Some(Repeat::One));
        assert_eq!(repeat_from(0), None);
    }

    #[tokio::test]
    async fn an_album_by_name_plays_the_album() {
        let (db, _d) = library().await;
        let p = played(&db, &ask("Watch the Throne", SiriKind::Album));
        assert_eq!(p.context, context(ContextKind::Album, "20"));
        assert_eq!(p.title, "Watch the Throne by Kanye West");
        // Siri didn't say what it is: the best match still wins.
        let p = played(&db, &ask("currents", SiriKind::Unknown));
        assert_eq!(p.context, context(ContextKind::Album, "21"));
    }

    #[tokio::test]
    async fn a_song_plays_within_its_album_from_that_song() {
        let (db, _d) = library().await;
        let p = played(&db, &ask("let it happen", SiriKind::Song));
        assert_eq!(p.context, context(ContextKind::Album, "21"));
        assert_eq!(p.start_track_id.as_deref(), Some("32"));
    }

    #[tokio::test]
    async fn an_artist_on_shuffle() {
        let (db, _d) = library().await;
        // "Play Kanye West on shuffle on Platter".
        let p = played(&db, &SiriAsk { name: Some("Kanye West".into()), kind: SiriKind::Artist, shuffle: true, ..Default::default() });
        assert_eq!(p.context, context(ContextKind::Artist, "10"));
        assert!(p.shuffle);
        assert_eq!(p.queue, SiriQueue::Now);
    }

    #[tokio::test]
    async fn artists_and_playlists() {
        let (db, _d) = library().await;
        let p = played(&db, &ask("road trip", SiriKind::Playlist));
        assert_eq!(p.context, context(ContextKind::Playlist, "40"));
        // "Play something by Kanye West".
        let p = played(&db, &SiriAsk { artist: Some("Kanye West".into()), ..Default::default() });
        assert_eq!(p.context, context(ContextKind::Artist, "10"));
    }

    #[tokio::test]
    async fn liked_songs_and_everything() {
        let (db, _d) = library().await;
        for name in ["Liked Songs", "my liked songs", "favorites"] {
            let p = played(&db, &ask(name, SiriKind::Playlist));
            assert_eq!(p.context, context(ContextKind::Liked, "liked"), "{name}");
        }
        let p = played(&db, &SiriAsk { name: Some("my library".into()), shuffle: true, ..Default::default() });
        assert_eq!(p.context.kind, ContextKind::Tracks);
        assert!(p.shuffle);
    }

    #[tokio::test]
    async fn play_something_carries_on_or_plays_the_first_mix() {
        let (db, _d) = library().await;
        let paused = Situation { can_resume: true, ..Default::default() };
        assert_eq!(answer(&db, &SiriAsk::default(), &paused), Some(SiriAnswer::Resume));
        assert_eq!(answer(&db, &SiriAsk { resume: true, ..Default::default() }, &Situation::default()), Some(SiriAnswer::Resume));
        let p = played(&db, &SiriAsk::default());
        assert_eq!(p.context, context(ContextKind::Mix, "m1"));
        // "Shuffle some music": everything, shuffled.
        let p = played_at(&db, &SiriAsk { shuffle: true, ..Default::default() }, &paused);
        assert_eq!(p.context.kind, ContextKind::Tracks);
        assert!(p.shuffle);
    }

    #[tokio::test]
    async fn mixes_by_name_genre_and_mood() {
        let (db, _d) = library().await;
        assert_eq!(played(&db, &ask("Daily Mix 2", SiriKind::Playlist)).context, context(ContextKind::Mix, "m2"));
        assert_eq!(played(&db, &ask("my daily mix 2", SiriKind::Unknown)).context, context(ContextKind::Mix, "m2"));
        // "Play hip hop" as a genre, or as genre names on an open request.
        assert_eq!(played(&db, &ask("hip hop", SiriKind::Genre)).context, context(ContextKind::Mix, "m3"));
        let p = played(&db, &SiriAsk { genres: vec!["Hip-Hop/Rap".into(), "hiphop".into()], ..Default::default() });
        assert_eq!(p.context, context(ContextKind::Mix, "m3"));
        let p = played(&db, &SiriAsk { moods: vec!["chill".into()], ..Default::default() });
        assert_eq!(p.context, context(ContextKind::Mix, "m4"));
        assert_eq!(answer(&db, &ask("polka", SiriKind::Genre), &Situation::default()), None);
    }

    #[tokio::test]
    async fn the_newest_album() {
        let (db, _d) = library().await;
        let newest = SiriAsk { artist: Some("Kanye West".into()), kind: SiriKind::Album, sort: SiriSort::Newest, ..Default::default() };
        assert_eq!(played(&db, &newest).context, context(ContextKind::Album, "22"));
        // Siri heard the artist as the name.
        let newest = SiriAsk { sort: SiriSort::Newest, ..ask("Kanye West", SiriKind::Album) };
        assert_eq!(played(&db, &newest).context, context(ContextKind::Album, "22"));
    }

    #[tokio::test]
    async fn more_like_this_is_radio() {
        let (db, _d) = library().await;
        let p = played_at(&db, &SiriAsk { this: true, ..Default::default() }, &playing());
        assert_eq!(p.context, context(ContextKind::Radio, "track:32"));
        assert_eq!(p.title, "music like Let It Happen");
        let p = played(&db, &ask("Tame Impala", SiriKind::Station));
        assert_eq!(p.context, context(ContextKind::Radio, "artist:11"));
        // Nothing playing: like "play something".
        assert_eq!(played(&db, &SiriAsk { this: true, ..Default::default() }).context.kind, ContextKind::Mix);
    }

    #[tokio::test]
    async fn queue_and_repeat_come_along() {
        let (db, _d) = library().await;
        let p = played(&db, &SiriAsk { queue: SiriQueue::Next, ..ask("Otis", SiriKind::Song) });
        assert_eq!((p.queue, p.start_track_id.as_deref()), (SiriQueue::Next, Some("30")));
        let p = played(&db, &SiriAsk { queue: SiriQueue::Later, ..ask("Currents", SiriKind::Album) });
        assert_eq!(p.queue, SiriQueue::Later);
        // A song on repeat repeats that song; anything else repeats it all.
        let p = played(&db, &SiriAsk { repeat: Some(Repeat::All), ..ask("Otis", SiriKind::Song) });
        assert_eq!(p.repeat, Some(Repeat::One));
        let p = played(&db, &SiriAsk { repeat: Some(Repeat::All), ..ask("Currents", SiriKind::Album) });
        assert_eq!(p.repeat, Some(Repeat::All));
        assert_eq!(played(&db, &ask("Currents", SiriKind::Album)).repeat, None);
    }

    #[tokio::test]
    async fn with_no_signal_only_whats_on_the_phone() {
        let (db, _d) = library().await;
        let kept = Kept { items: [(crate::downloads::store::ItemKind::Album, "21".to_string())].into(), tracks: ["32".to_string()].into() };
        let off = Situation { offline: Some(kept), ..Default::default() };
        assert_eq!(played_at(&db, &ask("Currents", SiriKind::Album), &off).context, context(ContextKind::Album, "21"));
        assert_eq!(played_at(&db, &ask("let it happen", SiriKind::Song), &off).start_track_id.as_deref(), Some("32"));
        assert_eq!(answer(&db, &ask("Watch the Throne", SiriKind::Album), &off), None);
        assert_eq!(answer(&db, &ask("Otis", SiriKind::Song), &off), None);
        // "Play something": the downloads (as the caller draws them with no
        // signal), not a mix drawn from the server.
        let downloads = || Ok(PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec!["32".into()]) });
        let p = resolve(&db, &SiriAsk::default(), &off, downloads).unwrap();
        assert!(matches!(p, Some(SiriAnswer::Play(p)) if p.context.kind == ContextKind::Tracks && p.shuffle));
    }

    #[tokio::test]
    async fn nothing_by_that_name_is_none_or_the_artist() {
        let (db, _d) = library().await;
        assert_eq!(answer(&db, &ask("Bohemian Rhapsody", SiriKind::Song), &Situation::default()), None);
        let p = played(&db, &SiriAsk { name: Some("Unknown Song".into()), artist: Some("Tame Impala".into()), kind: SiriKind::Song, ..Default::default() });
        assert_eq!(p.context, context(ContextKind::Artist, "11"));
    }

    #[tokio::test]
    async fn a_slow_get_carries_on_after_siris_wait() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;
        let done = Arc::new(AtomicBool::new(false));
        let d = done.clone();
        let slow = async move {
            tokio::time::sleep(Duration::from_millis(150)).await;
            d.store(true, Ordering::SeqCst);
            Ok(true)
        };
        assert_eq!(get_within(slow, Duration::from_millis(20)).await, Getting::StillGoing);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(done.load(Ordering::SeqCst), "the tap wasn't cancelled");
        assert_eq!(get_within(async { Ok(true) }, Duration::from_secs(1)).await, Getting::Started);
        assert_eq!(get_within(async { Ok(false) }, Duration::from_secs(1)).await, Getting::NothingFits);
        let failed = async { Err(crate::error::AppError::Invalid("Lidarr can't be reached".into())) };
        assert_eq!(get_within(failed, Duration::from_secs(1)).await, Getting::Failed);
    }

    #[tokio::test]
    async fn likes_and_playlists_find_their_song() {
        let (db, _d) = library().await;
        let now = playing();
        let this = SiriAsk { this: true, ..Default::default() };
        assert_eq!(song_for(&db, &this, now.playing.as_ref()).unwrap(), Some(("32".into(), "Let It Happen".into())));
        assert_eq!(song_for(&db, &SiriAsk::default(), None).unwrap(), None, "nothing playing");
        let named = song_for(&db, &ask("the less i know the better", SiriKind::Song), now.playing.as_ref()).unwrap();
        assert_eq!(named.map(|s| s.0).as_deref(), Some("33"));
        assert_eq!(song_for(&db, &ask("Bohemian Rhapsody", SiriKind::Song), now.playing.as_ref()).unwrap(), None);
        assert_eq!(playlist_named(&db, "my road trip playlist").unwrap().map(|p| p.0).as_deref(), Some("40"));
        assert_eq!(playlist_named(&db, "Road Trip").unwrap().map(|p| p.0).as_deref(), Some("40"));
        assert_eq!(playlist_named(&db, "gym").unwrap(), None);
    }
}
