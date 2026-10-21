//! "Hey Siri, play … on Platter": what Siri understood, matched against the
//! library copy on the phone, so it works with no signal for what's there.
//! The iOS side (the intent handling) hands the request to [`resolve`] and
//! plays what comes back.

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
    /// Siri didn't say ("play Watch the Throne on Platter").
    #[default]
    Unknown,
}

impl SiriKind {
    /// From Intents' `INMediaItemType`: song 1, album 2, artist 3,
    /// playlist 5; anything else (genre, station, "music") is left to the
    /// search.
    pub fn from_media_type(t: isize) -> Self {
        match t {
            1 => SiriKind::Song,
            2 => SiriKind::Album,
            3 => SiriKind::Artist,
            5 => SiriKind::Playlist,
            _ => SiriKind::Unknown,
        }
    }
}

/// A play request as Siri understood it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SiriAsk {
    /// The item's name ("Watch the Throne", "Liked Songs"); None for "play
    /// some music on Platter".
    pub name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub kind: SiriKind,
    pub shuffle: bool,
}

/// What to play for it.
#[derive(Debug, Clone, PartialEq)]
pub struct SiriPlay {
    pub context: PlayContext,
    /// Start here (a song asked for by name, played within its album).
    pub start_track_id: Option<String>,
    pub shuffle: bool,
    /// What it is, for Siri to say back.
    pub title: String,
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

/// Matches the request against the library; None when nothing fits.
/// `shuffle_all` makes the "shuffle everything" context (a random draw, as
/// CarPlay's Shuffle my library does).
pub fn resolve(
    db: &DbHandle,
    ask: &SiriAsk,
    shuffle_all: impl FnOnce() -> AppResult<PlayContext>,
) -> AppResult<Option<SiriPlay>> {
    let name = ask.name.as_deref().map(str::trim).filter(|n| !n.is_empty());
    let everything = || -> AppResult<Option<SiriPlay>> {
        Ok(Some(SiriPlay { context: shuffle_all()?, start_track_id: None, shuffle: true, title: "your library".into() }))
    };
    let Some(name) = name else {
        // "Play some music on Platter": everything, shuffled. An artist on
        // its own ("play something by X") is handled below.
        if ask.artist.is_none() {
            return everything();
        }
        return by_artist(db, ask.artist.as_deref().unwrap_or_default(), ask.shuffle);
    };
    if is_liked(name) {
        return Ok(Some(SiriPlay {
            context: context(ContextKind::Liked, "liked"),
            start_track_id: None,
            shuffle: ask.shuffle,
            title: "your liked songs".into(),
        }));
    }
    if is_everything(name) && ask.kind != SiriKind::Song {
        return everything();
    }
    // The name, plus the artist when Siri heard one, is a library search.
    let query = match &ask.artist {
        Some(a) if !a.trim().is_empty() => format!("{name} {a}"),
        _ => name.to_string(),
    };
    let filter = match ask.kind {
        SiriKind::Song => SearchFilter::Songs,
        SiriKind::Album => SearchFilter::Albums,
        SiriKind::Artist => SearchFilter::Artists,
        SiriKind::Playlist => SearchFilter::Playlists,
        SiriKind::Unknown => SearchFilter::All,
    };
    let found = crate::db::search::search(db, &query, filter, Some(5))?;
    let pick = match ask.kind {
        SiriKind::Song => found.songs.first().map(|t| song(t, ask.shuffle)),
        SiriKind::Album => found.albums.first().map(|a| album(a, ask.shuffle)),
        SiriKind::Artist => found.artists.first().map(|a| artist(a, ask.shuffle)),
        SiriKind::Playlist => found.playlists.first().map(|p| playlist(p, ask.shuffle)),
        SiriKind::Unknown => {
            use crate::db::types::SearchKind;
            match found.top {
                Some(top) => match top.kind {
                    SearchKind::Album => top.album.as_ref().map(|a| album(a, ask.shuffle)),
                    SearchKind::Artist => top.artist.as_ref().map(|a| artist(a, ask.shuffle)),
                    SearchKind::Playlist => top.playlist.as_ref().map(|p| playlist(p, ask.shuffle)),
                    _ => top.track.as_ref().map(|t| song(t, ask.shuffle)),
                },
                None => None,
            }
        }
    };
    // A match found only through the artist's name isn't the thing asked
    // for: it has to share a word with the name.
    let pick = pick.filter(|p| shares_a_word(&p.matched, name));
    if pick.is_some() || ask.artist.is_none() {
        return Ok(pick.map(|p| p.play));
    }
    // Nothing by that name, but the artist is known: play the artist.
    by_artist(db, ask.artist.as_deref().unwrap_or_default(), ask.shuffle)
}

fn by_artist(db: &DbHandle, artist_name: &str, shuffle: bool) -> AppResult<Option<SiriPlay>> {
    let found = crate::db::search::search(db, artist_name, SearchFilter::Artists, Some(3))?;
    Ok(found.artists.first().map(|a| artist(a, shuffle).play))
}

/// A pick and the title it was matched on.
struct Picked {
    play: SiriPlay,
    matched: String,
}

/// Words too common to tell two titles apart.
const STOP_WORDS: &[&str] = &["the", "a", "an", "of", "and", "in", "on", "to", "my", "by", "for", "it", "is"];

fn shares_a_word(title: &str, name: &str) -> bool {
    let title = words(title);
    let title: Vec<&str> = title.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    words(name)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() > 1 && !STOP_WORDS.contains(w))
        .any(|w| title.contains(&w))
}

fn song(t: &crate::model::Track, shuffle: bool) -> Picked {
    Picked { matched: t.title.clone(), play: SiriPlay {
        context: context(ContextKind::Album, &t.album_id),
        start_track_id: Some(t.id.clone()),
        shuffle,
        title: format!("{} by {}", t.title, t.track_artist.clone().unwrap_or_else(|| t.artist_title.clone())),
    } }
}

fn album(a: &crate::model::Album, shuffle: bool) -> Picked {
    Picked {
        matched: a.title.clone(),
        play: SiriPlay {
            context: context(ContextKind::Album, &a.id),
            start_track_id: None,
            shuffle,
            title: format!("{} by {}", a.title, a.artist_title),
        },
    }
}

fn artist(a: &crate::model::Artist, shuffle: bool) -> Picked {
    Picked {
        matched: a.title.clone(),
        play: SiriPlay { context: context(ContextKind::Artist, &a.id), start_track_id: None, shuffle, title: a.title.clone() },
    }
}

fn playlist(p: &crate::model::Playlist, shuffle: bool) -> Picked {
    Picked {
        matched: p.title.clone(),
        play: SiriPlay { context: context(ContextKind::Playlist, &p.id), start_track_id: None, shuffle, title: p.title.clone() },
    }
}

/// What to look for among music you don't have when the library has nothing
/// for a song Siri asked for ("play Middle Child on Platter"): the name,
/// and the artist when Siri heard one. None for albums, artists, playlists
/// and the library's own names.
pub fn catalogue_query(ask: &SiriAsk) -> Option<String> {
    if !matches!(ask.kind, SiriKind::Song | SiriKind::Unknown) {
        return None;
    }
    let name = ask.name.as_deref().map(str::trim).filter(|n| !n.is_empty())?;
    if is_liked(name) || is_everything(name) {
        return None;
    }
    Some(match ask.artist.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(a) => format!("{name} {a}"),
        None => name.to_string(),
    })
}

/// The first catalogue song that is the one asked for (it shares a word
/// with the name, and is by the artist when Siri heard one), as a tap.
pub fn tap_for(ask: &SiriAsk, songs: &[crate::discover::CatSong]) -> Option<crate::lidarr::tap::TapInput> {
    let name = ask.name.as_deref()?;
    let by = ask.artist.as_deref().map(words).filter(|a| !a.is_empty());
    let s = songs.iter().find(|s| shares_a_word(&s.title, name) && by.as_ref().is_none_or(|a| words(&s.artist).contains(a.as_str())))?;
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
        assert_eq!(catalogue_query(&asked("Revenge", None, SiriKind::Album)), None, "albums open their page instead");
        assert_eq!(catalogue_query(&asked("my liked songs", None, SiriKind::Song)), None);
        let songs = [cat("Childhood", "Someone Else"), cat("Middle Child", "J. Cole")];
        let t = tap_for(&asked("Middle Child", Some("J. Cole"), SiriKind::Song), &songs).unwrap();
        assert_eq!((t.title.as_str(), t.artist.as_str(), t.release_group_id.as_deref()), ("Middle Child", "J. Cole", Some("rg-Middle Child")));
        // Without an artist the first match wins; a wrong artist or name finds nothing.
        assert_eq!(tap_for(&asked("Middle Child", None, SiriKind::Song), &songs[1..]).unwrap().artist, "J. Cole");
        assert!(tap_for(&asked("Middle Child", Some("Drake"), SiriKind::Song), &songs).is_none());
        assert!(tap_for(&asked("Wet Dreamz", None, SiriKind::Song), &songs).is_none());
    }

    async fn library() -> (DbHandle, tempfile::TempDir) {
        let mut lib = FakeLibrary { sections: fake::generated(0, 0, 0).sections, ..Default::default() };
        let ye = fake::artist("10", "Kanye West");
        let tame = fake::artist("11", "Tame Impala");
        let wtt = fake::album("20", "Watch the Throne", &ye, 2011, Some("album"), &[]);
        let curr = fake::album("21", "Currents", &tame, 2015, Some("album"), &[]);
        lib.tracks.push(fake::track("30", "Otis", &wtt, 1, 1));
        lib.tracks.push(fake::track("31", "Niggas in Paris", &wtt, 1, 2));
        lib.tracks.push(fake::track("32", "Let It Happen", &curr, 1, 1));
        lib.tracks.push(fake::track("33", "The Less I Know the Better", &curr, 1, 2));
        lib.albums = vec![wtt, curr];
        lib.artists = vec![ye, tame];
        lib.playlists.push((fake::playlist("40", "Road Trip", 2), vec!["30".into(), "32".into()]));
        let dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(dir.path().join("library.db")).unwrap();
        let src = Arc::new(FakeSource::new(lib));
        db.sync_once(src as Arc<dyn LibrarySource>, true).await.unwrap();
        (db, dir)
    }

    fn all() -> AppResult<PlayContext> {
        Ok(PlayContext { kind: ContextKind::Tracks, id: None, track_ids: Some(vec!["30".into()]) })
    }

    fn ask(name: &str, kind: SiriKind) -> SiriAsk {
        SiriAsk { name: Some(name.into()), kind, ..Default::default() }
    }

    #[test]
    fn common_words_dont_make_a_match() {
        assert!(!shares_a_word("The Less I Know the Better", "The Wall"));
        assert!(shares_a_word("The Less I Know the Better", "less i know"));
        assert!(shares_a_word("Otis", "otis"));
    }

    #[test]
    fn siri_media_types_map_to_kinds() {
        assert_eq!(SiriKind::from_media_type(1), SiriKind::Song);
        assert_eq!(SiriKind::from_media_type(2), SiriKind::Album);
        assert_eq!(SiriKind::from_media_type(3), SiriKind::Artist);
        assert_eq!(SiriKind::from_media_type(5), SiriKind::Playlist);
        assert_eq!(SiriKind::from_media_type(0), SiriKind::Unknown);
        assert_eq!(SiriKind::from_media_type(18), SiriKind::Unknown);
    }

    #[tokio::test]
    async fn an_album_by_name_plays_the_album() {
        let (db, _d) = library().await;
        let p = resolve(&db, &ask("Watch the Throne", SiriKind::Album), all).unwrap().unwrap();
        assert_eq!(p.context, context(ContextKind::Album, "20"));
        assert_eq!(p.title, "Watch the Throne by Kanye West");
        // Siri didn't say what it is: the best match still wins.
        let p = resolve(&db, &ask("currents", SiriKind::Unknown), all).unwrap().unwrap();
        assert_eq!(p.context, context(ContextKind::Album, "21"));
    }

    #[tokio::test]
    async fn a_song_plays_within_its_album_from_that_song() {
        let (db, _d) = library().await;
        let p = resolve(&db, &ask("let it happen", SiriKind::Song), all).unwrap().unwrap();
        assert_eq!(p.context, context(ContextKind::Album, "21"));
        assert_eq!(p.start_track_id.as_deref(), Some("32"));
    }

    #[tokio::test]
    async fn artists_and_playlists() {
        let (db, _d) = library().await;
        let p = resolve(&db, &SiriAsk { name: Some("Tame Impala".into()), kind: SiriKind::Artist, shuffle: true, ..Default::default() }, all)
            .unwrap()
            .unwrap();
        assert_eq!(p.context, context(ContextKind::Artist, "11"));
        assert!(p.shuffle);
        let p = resolve(&db, &ask("road trip", SiriKind::Playlist), all).unwrap().unwrap();
        assert_eq!(p.context, context(ContextKind::Playlist, "40"));
        // "Play something by Kanye West".
        let p = resolve(&db, &SiriAsk { artist: Some("Kanye West".into()), ..Default::default() }, all).unwrap().unwrap();
        assert_eq!(p.context, context(ContextKind::Artist, "10"));
    }

    #[tokio::test]
    async fn liked_songs_and_everything() {
        let (db, _d) = library().await;
        for name in ["Liked Songs", "my liked songs", "favorites"] {
            let p = resolve(&db, &ask(name, SiriKind::Playlist), all).unwrap().unwrap();
            assert_eq!(p.context, context(ContextKind::Liked, "liked"), "{name}");
        }
        let p = resolve(&db, &SiriAsk::default(), all).unwrap().unwrap();
        assert_eq!(p.context.kind, ContextKind::Tracks, "play some music: everything");
        assert!(p.shuffle);
        let p = resolve(&db, &SiriAsk { name: Some("my library".into()), shuffle: true, ..Default::default() }, all).unwrap().unwrap();
        assert_eq!(p.context.kind, ContextKind::Tracks);
    }

    #[tokio::test]
    async fn nothing_by_that_name_is_none_or_the_artist() {
        let (db, _d) = library().await;
        assert_eq!(resolve(&db, &ask("Bohemian Rhapsody", SiriKind::Song), all).unwrap(), None);
        let p = resolve(
            &db,
            &SiriAsk { name: Some("Unknown Song".into()), artist: Some("Tame Impala".into()), kind: SiriKind::Song, ..Default::default() },
            all,
        )
        .unwrap()
        .unwrap();
        assert_eq!(p.context, context(ContextKind::Artist, "11"));
    }
}
