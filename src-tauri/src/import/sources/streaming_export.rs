//! A streaming service's account data download and its extended listening
//! history (zips of JSON files).
//!
//! Files are told apart by what is in them, not by their names:
//!
//! - library: an object with `tracks` (liked songs) and `albums` (saved
//!   albums), entries naming `artist`, `track`/`album` and a `uri`
//! - playlists: an object with `playlists`, each with a `name` and `items`
//!   whose `track` names `trackName`, `artistName`, `albumName`, `trackUri`
//!   (a local file has only a `localTrack` link with its names in it)
//! - history: an array of plays, either short (`endTime`, `artistName`,
//!   `trackName`, `msPlayed`) or extended (`ts`, `ms_played`,
//!   `master_metadata_track_name`, `master_metadata_album_artist_name`,
//!   `master_metadata_album_album_name`, plus episode fields and
//!   `incognito_mode`)
//!
//! Other names a field has gone by are accepted, unknown fields are
//! ignored, and other files in the download (account details and so on)
//! are never read past their shape.
//!
//! Privacy: from a history record only the song, the time and the time
//! played are copied. Network address, country, user agent and platform
//! fields are never read. Plays marked incognito are dropped; podcast and
//! audiobook records are skipped; records with no song name are counted
//! and skipped.

use serde_json::{Map, Value};

use super::{Parser, parse_time};
use crate::db::listens::SKIP_THRESHOLD_MS;
use crate::error::{AppError, AppResult};
use crate::import::files::{InputFile, decode_text};
use crate::import::{AlbumIn, Parsed, PlayIn, PlaylistIn, SongIn};

pub struct StreamingExport;

impl Parser for StreamingExport {
    fn parse(&self, files: &[InputFile]) -> AppResult<Parsed> {
        let mut out = Parsed::default();
        let mut usable = 0;
        for f in files.iter().filter(|f| f.ext() == "json") {
            // Plain UTF-8 is parsed in place, without a second copy as text.
            let v = match serde_json::from_slice::<Value>(&f.bytes) {
                Ok(v) => Ok(v),
                Err(_) => serde_json::from_str::<Value>(&decode_text(&f.bytes)),
            };
            let Ok(v) = v else {
                out.skipped.unknown_files += 1;
                continue;
            };
            if read_value(&v, &mut out) {
                usable += 1;
            } else {
                out.skipped.unknown_files += 1;
            }
        }
        if usable == 0 {
            return Err(AppError::Invalid(
                "No library, playlist or listening history files were found there. Pick the zip from the email, or the folder you unzipped it to."
                    .into(),
            ));
        }
        Ok(out)
    }
}

/// The first of these keys holding a non-empty string.
fn text(o: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| o.get(*k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from))
}

fn number(o: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|k| match o.get(*k)? {
        Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

fn flag(o: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|k| match o.get(*k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        _ => false,
    })
}

const TRACK_NAME: &[&str] = &["trackName", "track", "name", "track_name", "title", "master_metadata_track_name"];
const ARTIST_NAME: &[&str] = &["artistName", "artist", "artist_name", "master_metadata_album_artist_name"];
const ALBUM_NAME: &[&str] = &["albumName", "album", "album_name", "master_metadata_album_album_name"];
const TRACK_URI: &[&str] = &["trackUri", "uri", "track_uri"];
const ISRC: &[&str] = &["isrc", "ISRC"];
const DURATION: &[&str] = &["durationMs", "duration_ms", "duration"];

/// A song from an object naming one.
fn song(o: &Map<String, Value>) -> Option<SongIn> {
    Some(SongIn {
        title: text(o, TRACK_NAME)?,
        artist: text(o, ARTIST_NAME)?,
        album: text(o, ALBUM_NAME),
        duration_ms: number(o, DURATION).filter(|d| *d > 0),
        isrc: text(o, ISRC).and_then(|v| crate::tags::clean_isrc(&v)),
        service_id: text(o, TRACK_URI),
    })
}

/// Reads one file's JSON. False when it's nothing an import uses.
fn read_value(v: &Value, out: &mut Parsed) -> bool {
    match v {
        Value::Object(o) => {
            let mut used = false;
            if let Some(tracks) = o.get("tracks").and_then(Value::as_array) {
                used = true;
                let liked = out.liked.get_or_insert_with(Vec::new);
                for t in tracks.iter().filter_map(Value::as_object) {
                    match song(t) {
                        Some(s) => liked.push(s),
                        None => out.skipped.incomplete_rows += 1,
                    }
                }
            }
            if let Some(albums) = o.get("albums").and_then(Value::as_array) {
                used = true;
                let list = out.albums.get_or_insert_with(Vec::new);
                for a in albums.iter().filter_map(Value::as_object) {
                    match (text(a, ARTIST_NAME), text(a, &["album", "albumName", "name", "title"])) {
                        (Some(artist), Some(album)) => list.push(AlbumIn { artist, album }),
                        _ => out.skipped.incomplete_rows += 1,
                    }
                }
            }
            if let Some(lists) = o.get("playlists").and_then(Value::as_array) {
                used = true;
                for p in lists.iter().filter_map(Value::as_object) {
                    read_playlist(p, out);
                }
            }
            used
        }
        Value::Array(items) => {
            let records: Vec<&Map<String, Value>> = items.iter().filter_map(Value::as_object).collect();
            let is_history = records.iter().take(20).any(|r| {
                ["ts", "endTime", "ms_played", "msPlayed", "master_metadata_track_name", "endtime"].iter().any(|k| r.contains_key(*k))
            });
            if !is_history {
                return false;
            }
            for r in records {
                read_play(r, out);
            }
            true
        }
        _ => false,
    }
}

fn read_playlist(p: &Map<String, Value>, out: &mut Parsed) {
    let Some(name) = text(p, &["name", "title", "playlistName"]) else {
        out.skipped.incomplete_rows += 1;
        return;
    };
    let mut songs = Vec::new();
    let items = p.get("items").or_else(|| p.get("tracks")).and_then(Value::as_array);
    for it in items.into_iter().flatten().filter_map(Value::as_object) {
        // The song is under "track"; some versions put it on the item.
        let local = it.get("localTrack").and_then(Value::as_object);
        let t = it.get("track").and_then(Value::as_object).or(local);
        let is_episode = it.get("episode").is_some_and(|e| !e.is_null()) || it.get("audiobook").is_some_and(|e| !e.is_null());
        match t.and_then(song).or_else(|| song(it)).or_else(|| local.and_then(local_song)) {
            Some(s) => songs.push(s),
            None if is_episode => out.skipped.podcasts += 1,
            None => out.skipped.incomplete_rows += 1,
        }
    }
    out.playlists.push(PlaylistIn { name, songs });
}

/// A local file added to a playlist, which the download names only by its
/// link: `<service>:local:<artist>:<album>:<title>:<seconds>`, each part
/// percent-encoded with + for a space. Parts can be empty; one with no
/// title or no artist is nothing to match.
fn local_song(o: &Map<String, Value>) -> Option<SongIn> {
    let uri = text(o, &["uri", "trackUri"])?;
    let parts: Vec<&str> = uri.splitn(6, ':').collect();
    let [_, "local", artist, album, title, secs] = parts.as_slice() else { return None };
    let part = |s: &str| -> Option<String> {
        let s = percent_encoding::percent_decode_str(&s.replace('+', " ")).decode_utf8_lossy().trim().to_string();
        (!s.is_empty()).then_some(s)
    };
    Some(SongIn {
        artist: part(artist)?,
        title: part(title)?,
        album: part(album),
        duration_ms: secs.trim().parse::<u64>().ok().filter(|s| *s > 0).and_then(|s| s.checked_mul(1000)),
        isrc: None,
        service_id: Some(uri),
    })
}

/// One history record. Only the song, end time and time played are copied
/// out; every other field is left where it is.
fn read_play(r: &Map<String, Value>, out: &mut Parsed) {
    if flag(r, &["incognito_mode", "incognito", "private_session"]) {
        out.skipped.private_plays += 1;
        return;
    }
    let episode = ["episode_name", "episode_show_name", "audiobook_title", "audiobook_uri", "audiobook_chapter_title", "podcastName", "episodeName"]
        .iter()
        .any(|k| r.get(*k).is_some_and(|v| !v.is_null()));
    let title = text(r, &["master_metadata_track_name", "trackName", "track_name"]);
    let artist = text(r, &["master_metadata_album_artist_name", "artistName", "artist_name"]);
    let (Some(title), Some(artist)) = (title, artist) else {
        if episode {
            out.skipped.podcasts += 1;
        } else {
            out.skipped.no_metadata += 1;
        }
        return;
    };
    let Some(ended_at) = text(r, &["ts", "endTime", "end_time", "timestamp"]).and_then(|t| parse_time(&t)) else {
        out.skipped.incomplete_rows += 1;
        return;
    };
    let ms_played = number(r, &["ms_played", "msPlayed"]).unwrap_or(0);
    if ms_played < SKIP_THRESHOLD_MS {
        out.skipped.short_plays += 1;
        return;
    }
    out.plays.push(PlayIn {
        ended_at,
        ms_played,
        song: SongIn {
            artist,
            title,
            album: text(r, &["master_metadata_album_album_name", "albumName", "album_name"]),
            duration_ms: None,
            isrc: None,
            service_id: text(r, &["trackUri", "track_uri"]),
        },
    });
}
