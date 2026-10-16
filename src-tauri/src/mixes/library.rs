//! The library as the mix engine sees it: every track with its artist,
//! tags and listening signals, loaded from the cache in one pass and kept in
//! memory (indices instead of ids, so scoring 100k tracks stays cheap).
//!
//! Signals per track:
//! - plays: the larger of Plex's play count and every play in
//!   `db::listens::ALL_PLAYS` (the listen log and plays brought in from the
//!   server's history and other apps; a skip is never a play);
//! - skips: plays stopped inside 30 s, which count against the track;
//! - liked: rated at or above the liked rating;
//! - tags: Plex genres, styles and moods of the track and its album.
//!
//! Co-listening comes from the local listen log: songs (and their artists)
//! played near each other in one listening session.

use std::collections::{HashMap, HashSet};

use rusqlite::Connection;

use crate::error::AppResult;

use super::MixOptions;

/// Longest pause inside one listening session (same rule as Recents).
const SESSION_GAP_MS: i64 = crate::db::history::SESSION_GAP_MS;
/// Plays this close together in a session count as listened together.
const CO_WINDOW: usize = 4;
/// A Plex history entry this close to a local play of the same song is that
/// play reported to Plex, not another one.
const SAME_PLAY_MS: i64 = 10 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagKind {
    Genre,
    Style,
    Mood,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArtistRow {
    pub id: String,
    pub title: String,
    pub thumb: Option<String>,
    /// Plex similar artists (names), when the artist page has fetched them.
    pub similar: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackRow {
    pub id: String,
    pub artist: u32,
    pub album: u32,
    pub track_number: u32,
    pub duration_ms: u64,
    /// Tag ids, track and album tags together, no repeats.
    pub tags: Vec<u16>,
    pub liked: bool,
    pub plays: u32,
    pub skips: u32,
    /// Newest play, unix ms.
    pub last_played_ms: Option<i64>,
    /// May go in a mix (not hidden by the explicit filter, fan edit setting,
    /// exclusions, the unreleased setting or Hide).
    pub eligible: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlbumRow {
    pub id: String,
    pub thumb: Option<String>,
    /// Unix seconds.
    pub added_at: i64,
}

/// One play, oldest first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayEvent {
    pub track: u32,
    pub at_ms: i64,
    pub skipped: bool,
}

#[derive(Debug, Default)]
pub struct Lib {
    pub tracks: Vec<TrackRow>,
    pub artists: Vec<ArtistRow>,
    pub albums: Vec<AlbumRow>,
    pub tags: Vec<String>,
    pub tag_kinds: Vec<TagKind>,
    pub plays: Vec<PlayEvent>,
    pub index: HashMap<String, u32>,
    pub by_artist: Vec<Vec<u32>>,
    pub by_tag: Vec<Vec<u32>>,
    /// Tracks played near each other, both ways.
    pub co_tracks: HashMap<u32, Vec<(u32, f32)>>,
    /// Artists played near each other (smaller index first).
    pub co_artists: HashMap<(u32, u32), f32>,
}

/// Raw rows, so tests and the benchmark can build a library without SQLite.
#[derive(Debug, Default, Clone)]
pub struct Raw {
    pub artists: Vec<ArtistRow>,
    /// id, artist id, thumb, added_at, tags (kind, name), unreleased.
    pub albums: Vec<RawAlbum>,
    pub tracks: Vec<RawTrack>,
    /// Local plays: track id, started ms, skipped.
    pub listens: Vec<(String, i64, bool)>,
    /// Plays from elsewhere (the server's history, Tautulli, Last.fm, a
    /// history file): track id, played ms.
    pub history: Vec<(String, i64)>,
}

#[derive(Debug, Default, Clone)]
pub struct RawAlbum {
    pub id: String,
    pub artist_id: String,
    pub thumb: Option<String>,
    pub added_at: i64,
    pub tags: Vec<(TagKind, String)>,
    pub unreleased: bool,
}

#[derive(Debug, Default, Clone)]
pub struct RawTrack {
    pub id: String,
    pub artist_id: String,
    pub album_id: String,
    pub track_number: u32,
    pub duration_ms: u64,
    /// Plex genre and mood tags on the track itself.
    pub tags: Vec<String>,
    pub rating: Option<f64>,
    pub view_count: u32,
    /// Unix seconds.
    pub last_viewed_at: Option<i64>,
    pub explicit: bool,
    pub edit: bool,
    pub unreleased: bool,
}

fn json_vec(s: &str) -> Vec<String> {
    serde_json::from_str(s).unwrap_or_default()
}

/// Read everything the engine needs from the cache.
pub fn read_raw(c: &Connection) -> AppResult<Raw> {
    let mut raw = Raw::default();
    {
        let mut st = c.prepare("SELECT id, title, thumb, similar FROM artists")?;
        let rows = st.query_map([], |r| {
            let similar: String = r.get(3)?;
            let names = serde_json::from_str::<Vec<crate::db::SimilarArtist>>(&similar)
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.name)
                .collect();
            Ok(ArtistRow { id: r.get(0)?, title: r.get(1)?, thumb: r.get(2)?, similar: names })
        })?;
        for a in rows {
            raw.artists.push(a?);
        }
    }
    {
        let mut st = c.prepare(
            "SELECT al.id, al.artist_id, al.thumb, al.added_at, al.genres, al.styles, al.moods, al.format, al.subformats, al.studio,
                    at.version, al.matched, mc.found
             FROM albums al LEFT JOIN album_tags at ON at.album_id = al.id
             LEFT JOIN album_mb_check mc ON mc.album_id = al.id",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let mut tags = Vec::new();
            for g in json_vec(&r.get::<_, String>(4)?) {
                tags.push((TagKind::Genre, g));
            }
            for s in json_vec(&r.get::<_, String>(5)?) {
                tags.push((TagKind::Style, s));
            }
            for m in json_vec(&r.get::<_, String>(6)?) {
                tags.push((TagKind::Mood, m));
            }
            let format: Option<String> = r.get(7)?;
            let subformats = json_vec(&r.get::<_, String>(8)?);
            let studio: Option<String> = r.get(9)?;
            let version: Option<String> = r.get(10)?;
            let matched: Option<bool> = r.get(11)?;
            let mb: Option<bool> = r.get(12)?;
            let id: String = r.get(0)?;
            raw.albums.push(RawAlbum {
                unreleased: crate::db::release::album_unreleased(
                    &id,
                    format.as_deref(),
                    &subformats,
                    matched,
                    studio.as_deref(),
                    version.as_deref(),
                    mb,
                ),
                id,
                artist_id: r.get(1)?,
                thumb: r.get(2)?,
                added_at: r.get(3)?,
                tags,
            });
        }
    }
    {
        let mut st = c.prepare(
            "SELECT t.id, t.artist_id, t.album_id, t.track_number, t.duration_ms, t.genres, t.user_rating,
                    t.view_count, t.last_viewed_at, f.explicit, f.flags, f.version
             FROM tracks t LEFT JOIN file_tags f ON f.part_key = t.part_key AND f.status = 'ok'",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let flags: Option<String> = r.get(10)?;
            let version: Option<String> = r.get(11)?;
            let id: String = r.get(0)?;
            // A song's own mark decides for it; its album's status comes from the album row.
            let marked = crate::db::release::mark_of(&crate::db::release::mark_key("track", &id));
            let tagged = version.as_deref().is_some_and(crate::db::release::version_is_unreleased);
            let unreleased = match (version.as_deref().filter(|v| !v.trim().is_empty()), marked) {
                (Some(_), _) => tagged,
                (None, Some(m)) => m == crate::db::release::Mark::Unreleased,
                (None, None) => false,
            };
            raw.tracks.push(RawTrack {
                id,
                artist_id: r.get(1)?,
                album_id: r.get(2)?,
                track_number: r.get(3)?,
                duration_ms: r.get::<_, i64>(4)?.max(0) as u64,
                tags: json_vec(&r.get::<_, String>(5)?),
                rating: r.get(6)?,
                view_count: r.get(7)?,
                last_viewed_at: r.get(8)?,
                explicit: r.get::<_, Option<i64>>(9)? == Some(1),
                edit: crate::tags::is_edit(flags.as_deref()),
                unreleased,
            });
        }
    }
    {
        // Every play from the one combined view: local plays, and plays
        // brought in from the server's history, Tautulli, Last.fm or a file
        // (already without the ones that are a local play reported there).
        let mut st = c.prepare(&format!(
            "WITH plays AS ({}) SELECT source, rating_key, played_at, skipped FROM plays
             WHERE rating_key IS NOT NULL ORDER BY played_at, source, source_id",
            crate::db::listens::ALL_PLAYS
        ))?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?)))?;
        for row in rows {
            let (source, id, at, skipped) = row?;
            if source == "local" {
                raw.listens.push((id, at, skipped));
            } else if !skipped {
                raw.history.push((id, at));
            }
        }
    }
    {
        // The recent server history kept by the sync stands in until the
        // full history has been brought in (and not while it's turned off).
        let mut st = c.prepare(
            "SELECT rating_key, viewed_at FROM play_history
             WHERE NOT EXISTS (SELECT 1 FROM imported_plays WHERE source = 'plex_history')
               AND NOT EXISTS (SELECT 1 FROM kv WHERE key = ?1 AND value LIKE '%\"plex_history\"%')
             ORDER BY viewed_at",
        )?;
        let rows = st.query_map([crate::db::listens::EXCLUDED_KEY], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? * 1000)))?;
        for row in rows {
            raw.history.push(row?);
        }
    }
    Ok(raw)
}

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

impl Lib {
    /// Build the snapshot. `opts` decides which tracks may go in mixes.
    pub fn build(raw: Raw, opts: &MixOptions, liked_min: f64) -> Lib {
        let mut lib = Lib::default();
        let mut artist_ix: HashMap<String, u32> = HashMap::with_capacity(raw.artists.len());
        for a in raw.artists {
            artist_ix.insert(a.id.clone(), lib.artists.len() as u32);
            lib.artists.push(a);
        }
        let mut tag_ix: HashMap<String, u16> = HashMap::new();
        let mut add_tag = |lib: &mut Lib, kind: TagKind, name: &str| -> Option<u16> {
            let name = name.trim();
            if name.is_empty() {
                return None;
            }
            let key = fold(name);
            if let Some(&i) = tag_ix.get(&key) {
                // A name seen as a mood stays one (track tags mix both).
                if kind == TagKind::Mood {
                    lib.tag_kinds[i as usize] = TagKind::Mood;
                }
                return Some(i);
            }
            if lib.tags.len() >= u16::MAX as usize {
                return None;
            }
            let i = lib.tags.len() as u16;
            lib.tags.push(name.to_string());
            lib.tag_kinds.push(kind);
            tag_ix.insert(key, i);
            Some(i)
        };
        // Album tags first, so moods are known before track tags arrive.
        let mut album_ix: HashMap<String, u32> = HashMap::with_capacity(raw.albums.len());
        let mut album_tags: Vec<Vec<u16>> = Vec::with_capacity(raw.albums.len());
        let mut album_unreleased: Vec<bool> = Vec::with_capacity(raw.albums.len());
        for a in raw.albums {
            let mut tags = Vec::new();
            for (kind, name) in &a.tags {
                if let Some(t) = add_tag(&mut lib, *kind, name)
                    && !tags.contains(&t)
                {
                    tags.push(t);
                }
            }
            album_ix.insert(a.id.clone(), lib.albums.len() as u32);
            album_tags.push(tags);
            album_unreleased.push(a.unreleased);
            lib.albums.push(AlbumRow { id: a.id, thumb: a.thumb, added_at: a.added_at });
        }
        let ex_artists: HashSet<&str> = opts.exclude_artists.iter().map(String::as_str).collect();
        let ex_albums: HashSet<&str> = opts.exclude_albums.iter().map(String::as_str).collect();
        let ex_genres: HashSet<String> = opts.exclude_genres.iter().map(|g| fold(g)).collect();
        let view = opts.tag_view;
        for t in raw.tracks {
            let artist = match artist_ix.get(&t.artist_id) {
                Some(&a) => a,
                None => {
                    // An artist the cache doesn't list (shouldn't happen): keep the song anyway.
                    let a = lib.artists.len() as u32;
                    artist_ix.insert(t.artist_id.clone(), a);
                    lib.artists.push(ArtistRow { id: t.artist_id.clone(), title: String::new(), thumb: None, similar: vec![] });
                    a
                }
            };
            let album = match album_ix.get(&t.album_id) {
                Some(&a) => a,
                None => {
                    let a = lib.albums.len() as u32;
                    album_ix.insert(t.album_id.clone(), a);
                    album_tags.push(Vec::new());
                    album_unreleased.push(false);
                    lib.albums.push(AlbumRow { id: t.album_id.clone(), thumb: None, added_at: 0 });
                    a
                }
            };
            let mut tags = album_tags[album as usize].clone();
            for name in &t.tags {
                if let Some(tag) = add_tag(&mut lib, TagKind::Genre, name)
                    && !tags.contains(&tag)
                {
                    tags.push(tag);
                }
            }
            let unreleased = t.unreleased || album_unreleased[album as usize];
            let excluded_genre = !ex_genres.is_empty() && tags.iter().any(|g| ex_genres.contains(&fold(&lib.tags[*g as usize])));
            let eligible = !(view.filter_explicit && t.explicit)
                && (view.show_edits || !t.edit)
                && (opts.include_unreleased || !unreleased)
                && !ex_artists.contains(t.artist_id.as_str())
                && !ex_albums.contains(t.album_id.as_str())
                && !excluded_genre
                && !opts.hidden.hides(&t.id, &t.artist_id);
            lib.index.insert(t.id.clone(), lib.tracks.len() as u32);
            lib.tracks.push(TrackRow {
                id: t.id,
                artist,
                album,
                track_number: t.track_number,
                duration_ms: t.duration_ms,
                tags,
                liked: t.rating.is_some_and(|r| r >= liked_min),
                plays: t.view_count,
                skips: 0,
                last_played_ms: t.last_viewed_at.map(|s| s * 1000),
                eligible,
            });
        }
        lib.read_plays(raw.listens, raw.history);
        lib.by_artist = vec![Vec::new(); lib.artists.len()];
        lib.by_tag = vec![Vec::new(); lib.tags.len()];
        for (i, t) in lib.tracks.iter().enumerate() {
            lib.by_artist[t.artist as usize].push(i as u32);
            for g in &t.tags {
                lib.by_tag[*g as usize].push(i as u32);
            }
        }
        lib
    }

    /// Plays from the listen log and Plex history: counts, skips, last play
    /// and co-listening.
    fn read_plays(&mut self, mut listens: Vec<(String, i64, bool)>, history: Vec<(String, i64)>) {
        listens.sort_by_key(|l| l.1);
        let mut local_at: HashMap<u32, Vec<i64>> = HashMap::new();
        let mut counted = vec![0u32; self.tracks.len()];
        let mut events = Vec::with_capacity(listens.len() + history.len());
        for (id, at, skipped) in &listens {
            let Some(&t) = self.index.get(id) else { continue };
            events.push(PlayEvent { track: t, at_ms: *at, skipped: *skipped });
            if *skipped {
                self.tracks[t as usize].skips += 1;
            } else {
                counted[t as usize] += 1;
                local_at.entry(t).or_default().push(*at);
            }
        }
        // Co-listening: local plays only (Plex history has no sessions).
        let mut last_end = i64::MIN;
        let mut window: Vec<u32> = Vec::with_capacity(CO_WINDOW);
        for e in events.iter().filter(|e| !e.skipped) {
            if e.at_ms.saturating_sub(last_end) > SESSION_GAP_MS {
                window.clear();
            }
            last_end = e.at_ms;
            for &o in &window {
                if o == e.track {
                    continue;
                }
                let w = 1.0;
                bump(self.co_tracks.entry(o).or_default(), e.track, w);
                bump(self.co_tracks.entry(e.track).or_default(), o, w);
                let (a, b) = (self.tracks[o as usize].artist, self.tracks[e.track as usize].artist);
                if a != b {
                    *self.co_artists.entry((a.min(b), a.max(b))).or_default() += w;
                }
            }
            window.push(e.track);
            if window.len() > CO_WINDOW {
                window.remove(0);
            }
        }
        for (id, at) in &history {
            let Some(&t) = self.index.get(id) else { continue };
            let reported = local_at.get(&t).is_some_and(|v| v.iter().any(|l| (l - at).abs() <= SAME_PLAY_MS));
            if !reported {
                counted[t as usize] += 1;
                events.push(PlayEvent { track: t, at_ms: *at, skipped: false });
            }
        }
        events.sort_by_key(|e| e.at_ms);
        for e in &events {
            if !e.skipped {
                let tr = &mut self.tracks[e.track as usize];
                tr.last_played_ms = Some(tr.last_played_ms.map_or(e.at_ms, |l| l.max(e.at_ms)));
            }
        }
        for (t, n) in counted.into_iter().enumerate() {
            let tr = &mut self.tracks[t];
            tr.plays = tr.plays.max(n);
        }
        self.plays = events;
    }

    /// How much the user likes a track: plays and likes for it, quick skips
    /// against it.
    pub fn affinity(&self, t: u32) -> f32 {
        let tr = &self.tracks[t as usize];
        let plays = (1.0 + tr.plays as f32).ln();
        let liked = if tr.liked { 2.0 } else { 0.0 };
        plays + liked - 0.7 * tr.skips as f32
    }

    /// Played or liked enough to count as a favourite.
    pub fn familiar(&self, t: u32) -> bool {
        let tr = &self.tracks[t as usize];
        (tr.liked || tr.plays >= 2) && self.affinity(t) > 0.5
    }

    /// Hardly played and not skipped away: a candidate for something new.
    pub fn fresh(&self, t: u32) -> bool {
        let tr = &self.tracks[t as usize];
        tr.plays <= 1 && tr.skips < 2 && !tr.liked
    }

    pub fn track_artist_title(&self, t: u32) -> &str {
        &self.artists[self.tracks[t as usize].artist as usize].title
    }
}

fn bump(v: &mut Vec<(u32, f32)>, t: u32, w: f32) {
    match v.iter_mut().find(|(o, _)| *o == t) {
        Some(e) => e.1 += w,
        None => v.push((t, w)),
    }
}
