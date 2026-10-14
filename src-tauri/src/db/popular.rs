//! An artist's Popular list and the tracks that credit a name.
//!
//! Global popularity is a list of song titles in order, fetched by
//! `artistinfo` (ListenBrainz, or Last.fm top tracks when ListenBrainz has
//! nothing) and cached in the `artist_info` table under [`GLOBAL_ROW`].
//! Library tracks are matched to it by normalized title. Matched tracks come
//! first, in list order; the rest follow by your own plays, then most recent
//! play. Plex's `ratingCount` only breaks ties among unmatched tracks: it
//! exists for few tracks and isn't on one scale, so it can't rank.
//!
//! Credited artists: names in a track's artist line that have no library
//! artist get a generated page with the id `credit:<name, lower case>`.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::model::Track;

use super::credits;
use super::store::{TRACK_COLS, row_track};

/// Row name in `artist_info` for the global top songs.
pub const GLOBAL_ROW: &str = "listenbrainz";
/// Id prefix of a credited artist's page.
pub const CREDIT_PREFIX: &str = "credit:";
/// Songs kept from a global list.
pub const GLOBAL_KEEP: usize = 100;
/// Songs on the Popular list.
pub const POPULAR_N: usize = 10;
/// Two copies of one song: same title and length within this.
const SAME_SONG_MS: u64 = 2_000;

/// One song on a global list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSong {
    pub title: String,
    /// Listens (ListenBrainz) or plays (Last.fm), summed over duplicates.
    pub listens: u64,
}

/// An artist's global top songs, most popular first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct GlobalTop {
    /// "listenbrainz" or "lastfm".
    pub from: String,
    pub songs: Vec<GlobalSong>,
}

impl GlobalTop {
    /// Build from raw (title, listens) pairs: copies of a song (same
    /// normalized title) merge with their listens summed, then the list is
    /// sorted and cut to [`GLOBAL_KEEP`].
    pub fn from_pairs(from: &str, pairs: impl IntoIterator<Item = (String, u64)>) -> Self {
        let mut by_key: HashMap<String, usize> = HashMap::new();
        let mut songs: Vec<GlobalSong> = Vec::new();
        for (title, listens) in pairs {
            let key = normalize_title(&title);
            if key.is_empty() {
                continue;
            }
            match by_key.get(&key) {
                Some(&i) => songs[i].listens = songs[i].listens.saturating_add(listens),
                None => {
                    by_key.insert(key, songs.len());
                    songs.push(GlobalSong { title, listens });
                }
            }
        }
        // Stable: equal counts keep the source's order.
        songs.sort_by_key(|s| std::cmp::Reverse(s.listens));
        songs.truncate(GLOBAL_KEEP);
        Self { from: from.to_owned(), songs }
    }

    fn ranks(&self) -> HashMap<String, usize> {
        let mut out = HashMap::new();
        for (i, s) in self.songs.iter().enumerate() {
            out.entry(normalize_title(&s.title)).or_insert(i);
        }
        out
    }
}

/// Words after " - " that mark a version of the same song.
const VERSION_WORDS: &[&str] = &["remaster", "version", "mono", "stereo", "edit", "mix", "live", "demo", "single", "mixed"];

/// A title folded for matching: lower case, bracketed parts and featured
/// artists dropped, version suffixes after " - " cut ("2011 Remaster"),
/// apostrophes removed and other punctuation turned into single spaces.
pub fn normalize_title(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let lower: String = s.nfkc().collect::<String>().to_lowercase();
    // Bracketed parts: "(feat. X)", "[Bonus Track]", "(2011 Remaster)".
    let mut out = String::with_capacity(lower.len());
    let mut depth = 0usize;
    for ch in lower.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    // " - 2011 Remaster", " - Single Version", " - Live".
    if let Some(i) = out.find(" - ") {
        let tail = &out[i + 3..];
        if VERSION_WORDS.iter().any(|w| tail.split(|c: char| !c.is_alphanumeric()).any(|t| t == *w || t.starts_with("remaster"))) {
            out.truncate(i);
        }
    }
    // "Song feat. Someone".
    for sep in [" feat. ", " ft. ", " featuring "] {
        if let Some(i) = out.find(sep) {
            out.truncate(i);
        }
    }
    let mut folded = String::with_capacity(out.len());
    for ch in out.chars() {
        if ch == '\'' || ch == '\u{2019}' || ch == '`' {
            continue;
        }
        if ch == '&' {
            folded.push_str(" and ");
        } else if ch.is_alphanumeric() {
            folded.push(ch);
        } else {
            folded.push(' ');
        }
    }
    folded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The cached global list for a page id (stale or not), when there is one.
pub fn global_top(c: &Connection, id: &str) -> AppResult<Option<GlobalTop>> {
    let data: Option<Option<String>> = c
        .query_row(
            "SELECT data FROM artist_info WHERE artist_id = ?1 AND source = ?2",
            params![id, GLOBAL_ROW],
            |r| r.get(0),
        )
        .optional()?;
    Ok(data.flatten().and_then(|d| serde_json::from_str::<GlobalTop>(&d).ok()).filter(|g| !g.songs.is_empty()))
}

/// Rank candidates for Popular (see the module docs). `global` None ranks by
/// your plays only. Copies of a song show once, keeping the better ranked.
pub fn rank(mut cands: Vec<Track>, global: Option<&GlobalTop>, n: usize) -> Vec<Track> {
    let ranks = global.map(GlobalTop::ranks).unwrap_or_default();
    let keyed: Vec<(Option<usize>, String)> = cands
        .iter()
        .map(|t| {
            let k = normalize_title(&t.title);
            (ranks.get(&k).copied(), k)
        })
        .collect();
    let mut order: Vec<usize> = (0..cands.len()).collect();
    // Stable sort keeps library order for full ties.
    order.sort_by(|&a, &b| {
        let (ta, tb) = (&cands[a], &cands[b]);
        let (ra, rb) = (keyed[a].0, keyed[b].0);
        match (ra, rb) {
            (Some(x), Some(y)) => x.cmp(&y).then(tb.view_count.cmp(&ta.view_count)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => tb
                .view_count
                .cmp(&ta.view_count)
                .then(tb.last_viewed_at.cmp(&ta.last_viewed_at))
                .then(tb.rating_count.cmp(&ta.rating_count)),
        }
    });
    let mut picked: Vec<(usize, Option<usize>)> = Vec::with_capacity(n);
    for i in order {
        let t = &cands[i];
        let dupe = picked.iter().any(|&(j, r)| {
            let o = &cands[j];
            o.id == t.id
                || (r.is_some() && r == keyed[i].0)
                || (keyed[j].1 == keyed[i].1 && o.duration_ms.abs_diff(t.duration_ms) <= SAME_SONG_MS)
        });
        if !dupe {
            picked.push((i, keyed[i].0));
            if picked.len() == n {
                break;
            }
        }
    }
    let mut slots: Vec<Option<Track>> = cands.drain(..).map(Some).collect();
    picked.into_iter().filter_map(|(i, _)| slots[i].take()).collect()
}

/// Lower-cased names that split credits must keep whole: library artists and
/// learned names (`credit_names`) that appear in `line`.
pub fn known_in(c: &Connection, line: &str) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached(
        "SELECT lower(title) FROM artists WHERE instr(lower(?1), lower(title)) > 0
         UNION SELECT key FROM credit_names WHERE instr(lower(?1), key) > 0",
    )?;
    let rows = st.query_map([line], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The credited names of a track's artist line, split with the known names.
pub fn line_credits(c: &Connection, line: &str) -> AppResult<Vec<String>> {
    let known = known_in(c, line)?;
    Ok(credits::split_credits(line, |k| known.iter().any(|n| n == k)).into_iter().map(str::to_owned).collect())
}

/// Tracks whose artist line credits `name` as a whole name (any case),
/// leaving out tracks by `exclude_artist`. The search index finds lines that
/// mention the name without a table scan; names with no letters or digits
/// ("$") fall back to a scan.
pub fn credited_tracks(c: &Connection, name: &str, exclude_artist: Option<&str>) -> AppResult<Vec<Track>> {
    let me = name.trim().to_lowercase();
    if me.is_empty() {
        return Ok(Vec::new());
    }
    let exclude = exclude_artist.unwrap_or("");
    let mentioned: Vec<Track> = if me.chars().any(char::is_alphanumeric) {
        let phrase = format!("track_artist : \"{}\"", me.replace('"', "\"\""));
        let mut st = c.prepare_cached(&format!(
            "SELECT {TRACK_COLS} FROM tracks_fts JOIN tracks t ON t.pk = tracks_fts.rowid
             WHERE tracks_fts MATCH ?1 AND t.artist_id <> ?2 ORDER BY t.pk"
        ))?;
        st.query_map(params![phrase, exclude], row_track)?.collect::<Result<_, _>>()?
    } else {
        let mut st = c.prepare_cached(&format!(
            "SELECT {TRACK_COLS} FROM tracks t
             WHERE t.track_artist IS NOT NULL AND instr(lower(t.track_artist), ?1) > 0 AND t.artist_id <> ?2
             ORDER BY t.pk"
        ))?;
        st.query_map(params![me, exclude], row_track)?.collect::<Result<_, _>>()?
    };
    let mut out = Vec::new();
    for t in mentioned {
        let line = t.track_artist.as_deref().unwrap_or_default();
        if line_credits(c, line)?.iter().any(|n| n.to_lowercase() == me) {
            out.push(t);
        }
    }
    Ok(out)
}

/// The credited name as written on the first track that credits it.
pub fn credited_display_name(c: &Connection, key: &str) -> AppResult<Option<String>> {
    let tracks = credited_tracks(c, key, None)?;
    for t in &tracks {
        if let Some(line) = &t.track_artist
            && let Some(n) = line_credits(c, line)?.into_iter().find(|n| n.to_lowercase() == key)
        {
            return Ok(Some(n));
        }
    }
    Ok(None)
}

/// `credit:<name>` for a credited name.
pub fn credit_id(name: &str) -> String {
    format!("{CREDIT_PREFIX}{}", name.trim().to_lowercase())
}

/// The lower-cased name in a `credit:` id.
pub fn credit_key(id: &str) -> Option<&str> {
    id.strip_prefix(CREDIT_PREFIX).map(str::trim).filter(|k| !k.is_empty())
}

/// Remember names that contain a separator ("Tyler, The Maker") so splits
/// keep them whole. Returns how many were new.
pub fn learn_names<'a>(c: &Connection, names: impl IntoIterator<Item = &'a str>) -> AppResult<usize> {
    let mut st = c.prepare_cached("INSERT OR IGNORE INTO credit_names (key, name) VALUES (?1, ?2)")?;
    let mut n = 0;
    for name in names {
        let name = name.trim();
        if name.is_empty() || !credits::has_separator(name) {
            continue;
        }
        n += st.execute(params![name.to_lowercase(), name])?;
    }
    Ok(n)
}

/// Learned names with no library artist of the same name.
pub fn learned_names(c: &Connection) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached(
        "SELECT name FROM credit_names cn
         WHERE NOT EXISTS (SELECT 1 FROM artists ar WHERE lower(ar.title) = cn.key) ORDER BY key",
    )?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(id: &str, title: &str, plays: u32, last: Option<i64>, count: Option<u32>) -> Track {
        let al = crate::db::fake::album("al", "LP", &crate::db::fake::artist("a", "A"), 2020, None, &[]);
        let mut t = crate::db::fake::track(id, title, &al, 1, 1);
        t.view_count = plays;
        t.last_viewed_at = last;
        t.rating_count = count;
        t
    }

    fn ids(v: &[Track]) -> Vec<&str> {
        v.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn normalizes_titles() {
        for (a, b) in [
            ("Song Title", "song title"),
            ("Song Title (feat. Someone)", "song title"),
            ("Song Title [Bonus Track]", "song title"),
            ("Song Title - 2011 Remaster", "song title"),
            ("Song Title - Remastered 2009", "song title"),
            ("Song Title - Single Version", "song title"),
            ("Song Title feat. Someone", "song title"),
            ("Don't Stop", "dont stop"),
            ("Don\u{2019}t  Stop!", "dont stop"),
            ("Rock & Roll", "rock and roll"),
            ("Ｗｉｄｅ", "wide"),
        ] {
            assert_eq!(normalize_title(a), normalize_title(b), "{a} vs {b}");
        }
        // A real subtitle after " - " stays.
        assert_ne!(normalize_title("Part One - The Return"), normalize_title("Part One"));
        assert_eq!(normalize_title("Ünïcode"), "ünïcode");
    }

    #[test]
    fn duplicate_recordings_merge_with_listens_summed() {
        let g = GlobalTop::from_pairs(
            "listenbrainz",
            [("Hit".into(), 100), ("Other".into(), 150), ("Hit - 2011 Remaster".into(), 80), ("hit (Live)".into(), 1)],
        );
        assert_eq!(
            g.songs,
            vec![GlobalSong { title: "Hit".into(), listens: 181 }, GlobalSong { title: "Other".into(), listens: 150 }]
        );
    }

    #[test]
    fn keeps_the_top_hundred() {
        let g = GlobalTop::from_pairs("listenbrainz", (0..250).map(|i| (format!("Song {i}"), 1000 - i as u64)));
        assert_eq!(g.songs.len(), GLOBAL_KEEP);
        assert_eq!(g.songs[0].title, "Song 0");
    }

    #[test]
    fn the_global_list_wins_over_rating_counts_and_plays() {
        // One album with huge rating counts; the global list ranks the others.
        let cands = vec![
            tr("new1", "Deep Cut", 0, None, Some(318_000)),
            tr("new2", "Filler", 0, None, Some(133_000)),
            tr("old1", "Famous Single", 2, None, Some(618)),
            tr("old2", "Fan Favourite", 0, None, None),
            tr("mine", "My Favourite", 50, Some(9), None),
        ];
        let g = GlobalTop::from_pairs("listenbrainz", [("Famous Single".into(), 900), ("Fan Favourite - Remastered".into(), 500), ("Deep Cut".into(), 10)]);
        assert_eq!(ids(&rank(cands.clone(), Some(&g), 10)), ["old1", "old2", "new1", "mine", "new2"]);
        // No list: plays, then last play; rating count only breaks ties.
        assert_eq!(ids(&rank(cands, None, 10)), ["mine", "old1", "new1", "new2", "old2"]);
    }

    #[test]
    fn copies_of_a_matched_song_show_once() {
        let mut a = tr("a", "Hit", 0, None, None);
        a.duration_ms = 200_000;
        let mut b = tr("b", "Hit (Single Edit)", 3, None, None);
        b.duration_ms = 180_000;
        let c = tr("c", "Other", 0, None, None);
        let g = GlobalTop::from_pairs("lastfm", [("Hit".into(), 9), ("Other".into(), 1)]);
        assert_eq!(ids(&rank(vec![a, b, c], Some(&g), 10)), ["b", "c"]);
    }

    #[test]
    fn credit_ids_round_trip() {
        assert_eq!(credit_id(" Some Guest "), "credit:some guest");
        assert_eq!(credit_key("credit:some guest"), Some("some guest"));
        assert_eq!(credit_key("credit:"), None);
        assert_eq!(credit_key("123"), None);
    }
}
