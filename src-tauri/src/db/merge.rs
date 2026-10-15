//! Music on more than one server.
//!
//! Each included library syncs into the same tables; ids carry their server
//! (`crate::sid`), so nothing collides. After a sync, [`rebuild`] finds the
//! albums, songs and artists that are the same as one in another library and
//! records each copy in `dupes` with the copy that stands for the group (its
//! "canon"): the one from the main library when there is one, else the best
//! quality. The "All servers" view ([`ServerView::All`]) leaves the other
//! copies out, so each album shows as one card. One server's view shows only
//! that server's rows.
//!
//! Albums are the same when Plex matched both to the same MusicBrainz
//! release, or else when the artist, title and track count match and every
//! song's length is within [`DURATION_SLACK_MS`] of the other's. Songs of
//! matching albums pair up by disc and track number (or order); other songs
//! match on their MusicBrainz track id. Artists match on their name.
//!
//! What plays is chosen when a song is queued ([`best_copies`]): the copy on
//! a server reached over the local network first, then remote access, then
//! the relay, and between those the better file (lossless, bit depth, sample
//! rate, bitrate). Plays, likes and play queues then go to that copy's
//! server, the one that has the file.

use std::collections::{HashMap, HashSet};

use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::error::AppResult;
use crate::model::Track;

/// Songs whose lengths differ by more than this aren't the same recording.
pub const DURATION_SLACK_MS: u64 = 2_000;

/// What the library shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ServerView {
    /// Every included server, the same music shown once.
    #[default]
    All,
    /// One server's music (its tag, `crate::sid`).
    Server(String),
}

impl ServerView {
    /// From the setting: "all", or a server tag.
    pub fn parse(s: &str) -> Self {
        match s {
            t if crate::sid::is_tag(t) => Self::Server(t.to_string()),
            _ => Self::All,
        }
    }

    pub fn as_setting(&self) -> String {
        match self {
            Self::All => "all".into(),
            Self::Server(t) => t.clone(),
        }
    }

    /// An SQL condition: the row `{alias}.id` of `kind` ("album", "track",
    /// "artist") shows in this view.
    pub fn clause(&self, alias: &str, kind: &str) -> String {
        match self {
            Self::All => format!("{alias}.id NOT IN (SELECT id FROM dupes WHERE kind = '{kind}')"),
            Self::Server(tag) => {
                // Tags are 8 hex digits (checked in parse), safe in SQL.
                let plain = if crate::sid::legacy().as_deref() == Some(tag.as_str()) {
                    format!(" OR instr({alias}.id, ':') = 0")
                } else {
                    String::new()
                };
                format!("(substr({alias}.id, 1, 9) = '{tag}:'{plain})")
            }
        }
    }
}

/// Which ids a view shows, for lists filtered after the query.
#[derive(Debug, Clone, Default)]
pub struct ViewFilter {
    view: ServerView,
    dupes: HashSet<String>,
}

impl ViewFilter {
    pub fn load(c: &Connection, view: &ServerView) -> AppResult<Self> {
        let dupes = match view {
            ServerView::All => {
                let mut st = c.prepare_cached("SELECT id FROM dupes")?;
                st.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?
            }
            ServerView::Server(_) => HashSet::new(),
        };
        Ok(Self { view: view.clone(), dupes })
    }

    /// Nothing is left out (one library, or no copies found).
    pub fn is_noop(&self) -> bool {
        matches!(self.view, ServerView::All) && self.dupes.is_empty()
    }

    /// Whether the item with this id shows. Ids that aren't Plex items
    /// (local playlists, mixes) always do.
    pub fn shows(&self, id: &str) -> bool {
        match &self.view {
            ServerView::All => !self.dupes.contains(id),
            ServerView::Server(tag) => match crate::sid::tag_of(id) {
                Some(t) => t == tag,
                None => !crate::sid::is_plain_key(id) || crate::sid::legacy().as_deref() == Some(tag.as_str()),
            },
        }
    }
}

impl ViewFilter {
    pub fn albums(&self, v: &mut Vec<crate::model::Album>) {
        v.retain(|a| self.shows(&a.id));
    }

    pub fn artists(&self, v: &mut Vec<crate::model::Artist>) {
        v.retain(|a| self.shows(&a.id));
    }

    pub fn tracks(&self, v: &mut Vec<Track>) {
        v.retain(|t| self.shows(&t.id));
    }

    pub fn playlists(&self, v: &mut Vec<crate::model::Playlist>) {
        v.retain(|p| self.shows(&p.id));
    }

    pub fn home_items(&self, v: &mut Vec<super::types::HomeItem>) {
        v.retain(|i| self.shows(i.key().1));
    }

    pub fn home(&self, d: &mut super::types::HomeData) {
        if self.is_noop() {
            return;
        }
        self.home_items(&mut d.shortcuts);
        self.home_items(&mut d.recently_played);
        self.albums(&mut d.jump_back_in);
        self.playlists(&mut d.playlists);
        self.albums(&mut d.recently_added);
        self.artists(&mut d.top_artists);
    }

    pub fn search(&self, r: &mut super::types::SearchResults) {
        if self.is_noop() {
            return;
        }
        self.tracks(&mut r.songs);
        self.artists(&mut r.artists);
        self.albums(&mut r.albums);
        self.playlists(&mut r.playlists);
        if let Some(top) = &r.top {
            let id = top
                .artist
                .as_ref()
                .map(|a| a.id.as_str())
                .or(top.album.as_ref().map(|a| a.id.as_str()))
                .or(top.track.as_ref().map(|t| t.id.as_str()))
                .or(top.playlist.as_ref().map(|p| p.id.as_str()));
            if id.is_some_and(|id| !self.shows(id)) {
                r.top = None;
            }
        }
    }

    pub fn library_items(&self, v: &mut Vec<super::types::LibraryItem>) {
        v.retain(|i| self.shows(&i.id));
    }

    /// An artist page in this view: one server's albums, or, across all
    /// servers, the albums of the same artist on the others too (each
    /// album once).
    pub fn artist_page(&self, c: &Connection, page: &mut super::types::ArtistPage) -> AppResult<()> {
        if self.is_noop() {
            return Ok(());
        }
        let mut extra = Vec::new();
        if matches!(self.view, ServerView::All) {
            let mut st = c.prepare_cached(&format!(
                "SELECT {} FROM albums al WHERE al.artist_id = ?1
                 ORDER BY al.section_rank, al.year IS NULL, al.year DESC, al.added_at DESC",
                super::store::ALBUM_COLS
            ))?;
            for other in copies_of(c, &page.artist.id)?.into_iter().filter(|id| *id != page.artist.id) {
                extra.extend(st.query_map([&other], super::store::row_album)?.collect::<Result<Vec<_>, _>>()?);
            }
        }
        let mut seen: HashSet<String> = HashSet::new();
        for s in &mut page.sections {
            s.albums.retain(|a| self.shows(&a.id) && seen.insert(a.id.clone()));
        }
        for a in extra.into_iter().filter(|a| self.shows(&a.id) && seen.insert(a.id.clone())) {
            match page.sections.iter_mut().find(|s| s.section == a.section) {
                Some(s) => s.albums.push(a),
                None => page.sections.push(super::types::ArtistSection { section: a.section, albums: vec![a] }),
            }
        }
        let order = |s: &crate::model::ReleaseSection| super::release::SECTION_ORDER.iter().position(|x| x == s).unwrap_or(usize::MAX);
        page.sections.retain(|s| !s.albums.is_empty());
        page.sections.sort_by_key(|s| order(&s.section));
        self.tracks(&mut page.popular);
        self.albums(&mut page.appears_on);
        self.artists(&mut page.similar);
        Ok(())
    }
}

/// How good a source a server is right now. Lower is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rank {
    Local = 0,
    Remote = 1,
    Relay = 2,
    /// Not reached yet, or offline.
    Unknown = 3,
}

impl Rank {
    pub fn of(kind: Option<crate::plex::types::ConnectionKind>) -> Self {
        use crate::plex::types::ConnectionKind as K;
        match kind {
            Some(K::Local) => Self::Local,
            Some(K::Remote) => Self::Remote,
            Some(K::Relay) => Self::Relay,
            None => Self::Unknown,
        }
    }
}

/// Lower-case letters and digits only: "The Album (Deluxe)" and "the album
/// deluxe" match.
pub fn norm(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        } else {
            space = true;
        }
    }
    out
}

/// Better files sort higher: lossless, then bit depth, sample rate, bitrate.
pub fn quality(codec: Option<&str>, bit_depth: Option<u32>, sample_rate: Option<u32>, bitrate: Option<u32>) -> (u8, u32, u32, u32) {
    let lossless = codec.is_some_and(|c| {
        matches!(c.to_ascii_lowercase().as_str(), "flac" | "alac" | "wav" | "aiff" | "pcm" | "ape" | "wv" | "dsd")
    });
    (lossless as u8, bit_depth.unwrap_or(0), sample_rate.unwrap_or(0), bitrate.unwrap_or(0))
}

#[derive(Debug, Clone)]
struct AlbumRow {
    id: String,
    src: String,
    mbid: Option<String>,
    key: String,
    durations: Vec<u64>,
    quality: (u8, u32, u32, u32),
}

#[derive(Debug, Clone)]
struct TrackRow {
    id: String,
    album_id: String,
    src: String,
    mbid: Option<String>,
    disc: u32,
    number: u32,
    duration: u64,
    quality: (u8, u32, u32, u32),
}

fn close(a: u64, b: u64) -> bool {
    a.abs_diff(b) <= DURATION_SLACK_MS
}

fn same_album(a: &AlbumRow, b: &AlbumRow) -> bool {
    if let (Some(x), Some(y)) = (&a.mbid, &b.mbid) {
        return x.eq_ignore_ascii_case(y);
    }
    if a.key != b.key {
        return false;
    }
    // Songs not synced yet (or none) leave the count to decide.
    if a.durations.is_empty() || b.durations.is_empty() {
        return true;
    }
    a.durations.len() == b.durations.len() && a.durations.iter().zip(&b.durations).all(|(x, y)| close(*x, *y))
}

/// The copy that stands for a group: the main library's, else the best file,
/// else the smallest id (stable between runs).
fn pick<'a, T>(items: &'a [&'a T], src: impl Fn(&T) -> &str, q: impl Fn(&T) -> (u8, u32, u32, u32), id: impl Fn(&T) -> &str) -> &'a T {
    items
        .iter()
        .copied()
        .min_by(|a, b| {
            let main = |x: &T| (src(x) != super::store::MAIN_SRC) as u8;
            main(a).cmp(&main(b)).then(q(b).cmp(&q(a))).then(id(a).cmp(id(b)))
        })
        .expect("a group has members")
}

/// Find the same albums, songs and artists across libraries and record the
/// copies in `dupes`. Clears it when there's only one library.
pub fn rebuild(tx: &Transaction<'_>) -> AppResult<usize> {
    tx.execute("DELETE FROM dupes", [])?;
    let libraries: i64 = tx.query_row("SELECT count(DISTINCT src) FROM albums", [], |r| r.get(0))?;
    if libraries < 2 {
        return Ok(0);
    }
    let tracks: Vec<TrackRow> = {
        let mut st = tx.prepare(
            "SELECT id, album_id, src, mbid, disc_number, track_number, duration_ms, codec, bit_depth, sample_rate, bitrate
             FROM tracks ORDER BY album_id, disc_number, track_number, pk",
        )?;
        st.query_map([], |r| {
            Ok(TrackRow {
                id: r.get(0)?,
                album_id: r.get(1)?,
                src: r.get(2)?,
                mbid: r.get::<_, Option<String>>(3)?.filter(|m| !m.is_empty()),
                disc: r.get(4)?,
                number: r.get(5)?,
                duration: r.get::<_, i64>(6)?.max(0) as u64,
                quality: quality(r.get::<_, Option<String>>(7)?.as_deref(), r.get(8)?, r.get(9)?, r.get(10)?),
            })
        })?
        .collect::<Result<_, _>>()?
    };
    let mut by_album: HashMap<&str, Vec<&TrackRow>> = HashMap::new();
    for t in &tracks {
        by_album.entry(t.album_id.as_str()).or_default().push(t);
    }
    let albums: Vec<AlbumRow> = {
        let mut st = tx.prepare("SELECT id, src, mbid, artist_title, title, track_count FROM albums ORDER BY pk")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, src, mbid, artist, title, count) = row?;
            let songs = by_album.get(id.as_str()).map(Vec::as_slice).unwrap_or(&[]);
            let best = songs.iter().map(|t| t.quality).max().unwrap_or_default();
            out.push(AlbumRow {
                key: format!("{}|{}|{}", norm(&artist), norm(&title), count),
                durations: songs.iter().map(|t| t.duration).collect(),
                quality: best,
                mbid: mbid.filter(|m| !m.is_empty()),
                id,
                src,
            });
        }
        out
    };

    // Albums: candidates share a key or an MBID; a group never holds two
    // albums of one library (editions there are the file tags' business).
    let mut buckets: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, a) in albums.iter().enumerate() {
        buckets.entry(format!("k:{}", a.key)).or_default().push(i);
        if let Some(m) = &a.mbid {
            buckets.entry(format!("m:{}", m.to_ascii_lowercase())).or_default().push(i);
        }
    }
    let mut group_of: Vec<Option<usize>> = vec![None; albums.len()];
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut keys: Vec<&String> = buckets.keys().collect();
    keys.sort();
    for k in keys {
        let members = &buckets[k];
        for (x, &i) in members.iter().enumerate() {
            for &j in &members[x + 1..] {
                if albums[i].src == albums[j].src || !same_album(&albums[i], &albums[j]) {
                    continue;
                }
                match (group_of[i], group_of[j]) {
                    (Some(_), Some(_)) => {}
                    (Some(g), None) => {
                        if groups[g].iter().all(|&m| albums[m].src != albums[j].src) {
                            groups[g].push(j);
                            group_of[j] = Some(g);
                        }
                    }
                    (None, Some(g)) => {
                        if groups[g].iter().all(|&m| albums[m].src != albums[i].src) {
                            groups[g].push(i);
                            group_of[i] = Some(g);
                        }
                    }
                    (None, None) => {
                        group_of[i] = Some(groups.len());
                        group_of[j] = Some(groups.len());
                        groups.push(vec![i, j]);
                    }
                }
            }
        }
    }

    let mut ins = tx.prepare_cached("INSERT OR REPLACE INTO dupes (id, kind, canon) VALUES (?1, ?2, ?3)")?;
    let mut n = 0;
    let mut paired: HashSet<&str> = HashSet::new();
    for g in &groups {
        let members: Vec<&AlbumRow> = g.iter().map(|&i| &albums[i]).collect();
        let canon = pick(&members, |a| &a.src, |a| a.quality, |a| &a.id);
        let canon_songs = by_album.get(canon.id.as_str()).map(Vec::as_slice).unwrap_or(&[]);
        for a in members.iter().filter(|a| a.id != canon.id) {
            n += ins.execute(params![a.id, "album", canon.id])?;
            let songs = by_album.get(a.id.as_str()).map(Vec::as_slice).unwrap_or(&[]);
            for (k, t) in songs.iter().enumerate() {
                let twin = canon_songs
                    .iter()
                    .find(|c| c.disc == t.disc && c.number == t.number && t.number > 0 && close(c.duration, t.duration))
                    .or_else(|| canon_songs.get(k).filter(|c| close(c.duration, t.duration)));
                if let Some(c) = twin {
                    n += ins.execute(params![t.id, "track", c.id])?;
                    paired.insert(t.id.as_str());
                    paired.insert(c.id.as_str());
                }
            }
        }
    }

    // Songs outside matching albums: the same MusicBrainz track id in
    // another library.
    let mut by_mbid: HashMap<String, Vec<&TrackRow>> = HashMap::new();
    for t in tracks.iter().filter(|t| !paired.contains(t.id.as_str())) {
        if let Some(m) = &t.mbid {
            by_mbid.entry(m.to_ascii_lowercase()).or_default().push(t);
        }
    }
    for list in by_mbid.values() {
        let mut seen_src: HashSet<&str> = HashSet::new();
        let members: Vec<&TrackRow> = list.iter().copied().filter(|t| seen_src.insert(t.src.as_str())).collect();
        if members.len() < 2 {
            continue;
        }
        let canon = pick(&members, |t| &t.src, |t| t.quality, |t| &t.id);
        for t in members.iter().filter(|t| t.id != canon.id && close(t.duration, canon.duration)) {
            n += ins.execute(params![t.id, "track", canon.id])?;
        }
    }

    // Artists: the same name in another library.
    let artists: Vec<(String, String, String)> = {
        let mut st = tx.prepare("SELECT id, src, title FROM artists ORDER BY pk")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?
    };
    let mut by_name: HashMap<String, Vec<&(String, String, String)>> = HashMap::new();
    for a in &artists {
        let k = norm(&a.2);
        if !k.is_empty() {
            by_name.entry(k).or_default().push(a);
        }
    }
    for list in by_name.values() {
        let mut seen_src: HashSet<&str> = HashSet::new();
        let members: Vec<&(String, String, String)> = list.iter().copied().filter(|a| seen_src.insert(a.1.as_str())).collect();
        if members.len() < 2 {
            continue;
        }
        let canon = pick(&members, |a| &a.1, |_| (0, 0, 0, 0), |a| &a.0);
        for a in members.iter().filter(|a| a.0 != canon.0) {
            n += ins.execute(params![a.0, "artist", canon.0])?;
        }
    }
    Ok(n)
}

/// The copy that stands for `id` (itself when it has none).
pub fn canon_of(c: &Connection, id: &str) -> AppResult<String> {
    Ok(c
        .query_row("SELECT canon FROM dupes WHERE id = ?1", [id], |r| r.get(0))
        .optional()?
        .unwrap_or_else(|| id.to_string()))
}

/// Every copy of the item `id` belongs to, the canon first.
pub fn copies_of(c: &Connection, id: &str) -> AppResult<Vec<String>> {
    let canon = canon_of(c, id)?;
    let mut st = c.prepare_cached("SELECT id FROM dupes WHERE canon = ?1 ORDER BY id")?;
    let mut out = vec![canon.clone()];
    out.extend(st.query_map([&canon], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?);
    Ok(out)
}

/// Swap each song for its best copy (see the module notes): the server
/// reached the best way, then the better file. `ranks` maps server tags to
/// how they're reached now; a server not in it counts as not reached yet.
pub fn best_copies(c: &Connection, tracks: &mut [Track], ranks: &HashMap<String, Rank>) -> AppResult<usize> {
    let any: bool = c.query_row("SELECT EXISTS (SELECT 1 FROM dupes WHERE kind = 'track')", [], |r| r.get(0))?;
    if !any {
        return Ok(0);
    }
    let rank = |id: &str| crate::sid::owner(id).and_then(|t| ranks.get(&t).copied()).unwrap_or(Rank::Unknown);
    let mut swapped = 0;
    for t in tracks.iter_mut() {
        let ids = copies_of(c, &t.id)?;
        if ids.len() < 2 {
            continue;
        }
        let copies = super::queries::tracks_by_ids(c, &ids)?;
        let best = copies.iter().min_by(|a, b| {
            rank(&a.id)
                .cmp(&rank(&b.id))
                .then(
                    quality(b.codec.as_deref(), b.bit_depth, b.sample_rate, b.bitrate)
                        .cmp(&quality(a.codec.as_deref(), a.bit_depth, a.sample_rate, a.bitrate)),
                )
                .then((a.id != t.id).cmp(&(b.id != t.id)))
        });
        if let Some(b) = best
            && b.id != t.id
        {
            *t = b.clone();
            swapped += 1;
        }
    }
    Ok(swapped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::db::store;

    fn db() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let mut c = schema::open_writer(&dir.path().join("t.db")).unwrap();
        schema::migrate(&mut c).unwrap();
        (dir, c)
    }

    const A: &str = "aaaaaaaa";
    const B: &str = "bbbbbbbb";

    #[allow(clippy::too_many_arguments)]
    fn album(c: &Connection, tag: &str, src: &str, id: &str, artist: &str, title: &str, count: u32, mbid: Option<&str>) {
        c.execute(
            "INSERT INTO albums (id, title, sort_title, artist_id, artist_title, track_count, src, mbid)
             VALUES (?1, ?2, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![format!("{tag}:{id}"), title, format!("{tag}:ar{artist}"), artist, count, src, mbid],
        )
        .unwrap();
        c.execute(
            "INSERT OR IGNORE INTO artists (id, title, sort_title, src) VALUES (?1, ?2, ?2, ?3)",
            params![format!("{tag}:ar{artist}"), artist, src],
        )
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn track(c: &Connection, tag: &str, src: &str, id: &str, album: &str, n: u32, ms: i64, codec: &str, mbid: Option<&str>) {
        c.execute(
            "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, track_number, duration_ms,
               part_key, codec, src, mbid)
             VALUES (?1, ?1, ?2, 'x', 'ar', 'A', ?3, ?4, '/p', ?5, ?6, ?7)",
            params![format!("{tag}:{id}"), format!("{tag}:{album}"), n, ms, codec, src, mbid],
        )
        .unwrap();
    }

    fn dupes(c: &Connection) -> Vec<(String, String, String)> {
        let mut st = c.prepare("SELECT id, kind, canon FROM dupes ORDER BY kind, id").unwrap();
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect()
    }

    fn rebuild_now(c: &mut Connection) -> usize {
        let tx = c.transaction().unwrap();
        let n = rebuild(&tx).unwrap();
        tx.commit().unwrap();
        n
    }

    #[test]
    fn the_same_album_on_two_servers_shows_once() {
        let (_d, mut c) = db();
        let b = store::src_of(B, "1");
        // Same artist, title (spelled a bit differently), count and lengths.
        album(&c, A, "", "10", "Artist", "The Album (Deluxe)", 2, None);
        track(&c, A, "", "11", "10", 1, 200_000, "mp3", None);
        track(&c, A, "", "12", "10", 2, 180_000, "mp3", None);
        album(&c, B, &b, "20", "ARTIST", "the album deluxe", 2, None);
        track(&c, B, &b, "21", "20", 1, 201_000, "flac", None);
        track(&c, B, &b, "22", "20", 2, 179_500, "flac", None);
        // Same name, different lengths: another recording, kept.
        album(&c, A, "", "30", "Artist", "Live", 1, None);
        track(&c, A, "", "31", "30", 1, 300_000, "mp3", None);
        album(&c, B, &b, "40", "Artist", "Live", 1, None);
        track(&c, B, &b, "41", "40", 1, 260_000, "mp3", None);
        assert!(rebuild_now(&mut c) > 0);
        let d = dupes(&c);
        let has = |id: &str, kind: &str, canon: &str| d.iter().any(|x| x.0 == id && x.1 == kind && x.2 == canon);
        assert!(has("bbbbbbbb:20", "album", "aaaaaaaa:10"), "the main library's copy stands for it: {d:?}");
        assert!(has("bbbbbbbb:21", "track", "aaaaaaaa:11"));
        assert!(has("bbbbbbbb:22", "track", "aaaaaaaa:12"));
        assert!(!d.iter().any(|x| x.0.ends_with(":40") || x.0.ends_with(":30")), "different lengths aren't the same album");
        assert!(
            has("bbbbbbbb:arARTIST", "artist", "aaaaaaaa:arArtist") || has("bbbbbbbb:arArtist", "artist", "aaaaaaaa:arArtist"),
            "{d:?}"
        );

        // All servers: one card. One server: just its own.
        let all = ServerView::All;
        let count = |v: &ServerView| -> i64 {
            c.query_row(&format!("SELECT count(*) FROM albums al WHERE {}", v.clause("al", "album")), [], |r| r.get(0)).unwrap()
        };
        assert_eq!(count(&all), 3);
        assert_eq!(count(&ServerView::Server(B.into())), 2);
        assert_eq!(count(&ServerView::Server(A.into())), 2);
        let f = ViewFilter::load(&c, &all).unwrap();
        assert!(!f.shows("bbbbbbbb:20") && f.shows("aaaaaaaa:10") && f.shows("local:p"));
        let f = ViewFilter::load(&c, &ServerView::Server(B.into())).unwrap();
        assert!(f.shows("bbbbbbbb:20") && !f.shows("aaaaaaaa:10") && f.shows("local:p"));
    }

    #[test]
    fn musicbrainz_ids_win_over_names() {
        let (_d, mut c) = db();
        let b = store::src_of(B, "1");
        album(&c, A, "", "10", "Artist", "Album", 1, Some("REL-1"));
        track(&c, A, "", "11", "10", 1, 100_000, "mp3", None);
        // Retitled on the other server but the same release.
        album(&c, B, &b, "20", "Artist", "Album [2009 Remaster]", 1, Some("rel-1"));
        track(&c, B, &b, "21", "20", 1, 100_500, "mp3", None);
        // Same name, another release.
        album(&c, B, &b, "30", "Artist", "Album", 1, Some("rel-2"));
        track(&c, B, &b, "31", "30", 1, 100_000, "mp3", None);
        // A loose song on both, matched by its track id.
        album(&c, A, "", "50", "Other", "Single", 1, None);
        track(&c, A, "", "51", "50", 1, 90_000, "mp3", Some("t-9"));
        album(&c, B, &b, "60", "Other", "Comp", 9, None);
        track(&c, B, &b, "61", "60", 4, 90_200, "flac", Some("t-9"));
        rebuild_now(&mut c);
        let d = dupes(&c);
        assert!(d.contains(&("bbbbbbbb:20".into(), "album".into(), "aaaaaaaa:10".into())), "{d:?}");
        assert!(!d.iter().any(|x| x.0 == "bbbbbbbb:30"));
        assert!(d.contains(&("bbbbbbbb:61".into(), "track".into(), "aaaaaaaa:51".into())));
    }

    #[test]
    fn one_library_has_no_copies() {
        let (_d, mut c) = db();
        album(&c, A, "", "10", "Artist", "Album", 1, None);
        album(&c, A, "", "20", "Artist", "Album", 1, None);
        assert_eq!(rebuild_now(&mut c), 0);
        assert!(dupes(&c).is_empty());
        assert!(ViewFilter::load(&c, &ServerView::All).unwrap().is_noop());
    }

    #[test]
    fn the_best_copy_plays_local_server_first_then_quality() {
        let (_d, mut c) = db();
        let b = store::src_of(B, "1");
        album(&c, A, "", "10", "Artist", "Album", 1, None);
        track(&c, A, "", "11", "10", 1, 100_000, "mp3", None);
        album(&c, B, &b, "20", "Artist", "Album", 1, None);
        track(&c, B, &b, "21", "20", 1, 100_000, "flac", None);
        rebuild_now(&mut c);
        let one = |c: &Connection, ranks: &HashMap<String, Rank>| {
            let mut t = crate::db::queries::tracks_by_ids(c, &["aaaaaaaa:11".to_string()]).unwrap();
            best_copies(c, &mut t, ranks).unwrap();
            t[0].id.clone()
        };
        // Both reached the same way: the lossless file.
        let same: HashMap<String, Rank> = [(A.to_string(), Rank::Remote), (B.to_string(), Rank::Remote)].into();
        assert_eq!(one(&c, &same), "bbbbbbbb:21");
        // The main server is on the local network, the other over the relay.
        let local: HashMap<String, Rank> = [(A.to_string(), Rank::Local), (B.to_string(), Rank::Relay)].into();
        assert_eq!(one(&c, &local), "aaaaaaaa:11");
        // The other server can't be reached: what was asked for plays.
        let gone: HashMap<String, Rank> = [(A.to_string(), Rank::Remote)].into();
        assert_eq!(one(&c, &gone), "aaaaaaaa:11");
        // Starting from the copy works the same.
        let mut t = crate::db::queries::tracks_by_ids(&c, &["bbbbbbbb:21".to_string()]).unwrap();
        best_copies(&c, &mut t, &local).unwrap();
        assert_eq!(t[0].id, "aaaaaaaa:11");
    }

    #[test]
    fn names_fold_case_and_punctuation() {
        assert_eq!(norm("The Album (Deluxe)"), "the album deluxe");
        assert_eq!(norm("  Beyoncé / Live!  "), "beyoncé live");
        assert!(quality(Some("FLAC"), Some(16), None, None) > quality(Some("mp3"), None, None, Some(320)));
        assert_eq!(ServerView::parse("all"), ServerView::All);
        assert_eq!(ServerView::parse("0a1b2c3d"), ServerView::Server("0a1b2c3d".into()));
        assert_eq!(ServerView::parse("x'; DROP"), ServerView::All);
    }
}
