//! Mix generation: pure functions over a [`Lib`] snapshot.
//!
//! - Daily Mix 1-6: the top artists grouped by shared tags, co-listening and
//!   Plex similar artists; each mix is mostly favourites from its group with
//!   less-played songs from the group and its close neighbours (the split
//!   follows the Variety setting, 70/30 when balanced). 50 songs. A song
//!   goes in one Daily Mix a day at most.
//! - Discover: 30 hardly played songs closest to what you play most.
//! - New in your library: songs from albums added in the last 30 days.
//! - On Repeat: your 30 most played songs of the last 30 days.
//! - Rediscover: songs you played a lot that haven't played in 6 months.
//! - Genre and mood mixes: one per Plex tag with at least 25 songs.
//! - daylist: what you play around this hour on this kind of day, named
//!   "<genres and moods> <weekday> <part of day>".
//! - All-Time Top Songs: your most played songs by Plex play count.
//!
//! Every list keeps an artist to two songs in a row at most. Randomness is
//! a hash of the day and the mix, so the same day gives the same mixes.

use std::collections::{HashMap, HashSet};

use super::library::{Lib, TagKind};
use super::{MixKind, Variety};

pub const DAILY_MAX: usize = 6;
pub const DAILY_LEN: usize = 50;
pub const SHORT_LEN: usize = 30;
pub const TAG_MIX_LEN: usize = 50;
pub const DAYLIST_LEN: usize = 50;
/// Most songs in All-Time Top Songs.
pub const TOP_SONGS_LEN: usize = 100;
/// A genre or mood needs this many songs for its own mix.
pub const TAG_MIX_MIN: usize = 25;
/// Most genre mixes and most mood mixes made.
pub const TAG_MIXES_MAX: usize = 12;
/// Fewest songs a mix needs to be shown.
pub const MIX_MIN: usize = 5;
/// Artists considered for Daily Mixes.
const DAILY_ARTISTS: usize = 30;
/// Joins a group when this similar to it on average.
const CLUSTER_JOIN: f32 = 0.3;
/// Neighbouring artists a Daily Mix borrows fresh songs from.
const NEIGHBOURS: usize = 8;
const NEIGHBOUR_MIN: f32 = 0.3;
const DAY_MS: i64 = 24 * 3600 * 1000;
const ON_REPEAT_WINDOW_MS: i64 = 30 * DAY_MS;
const NEW_WINDOW_S: i64 = 30 * 24 * 3600;
const REDISCOVER_AFTER_MS: i64 = 182 * DAY_MS;
const REDISCOVER_MIN_PLAYS: u32 = 3;
/// Most songs from one album in New in your library.
const NEW_PER_ALBUM: usize = 3;

/// Card colours, picked per mix (dark text sits on them).
pub const PALETTE: [&str; 10] = [
    "#f2a7c3", "#f5b971", "#8fc1f0", "#9fd8a6", "#c9b1f2", "#f2d16b", "#7fd6d0", "#f29b8f", "#b8d77a", "#e3b5e8",
];

/// What every generator needs besides the library.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub now_ms: i64,
    /// Minutes east of UTC (local time = UTC + offset).
    pub tz_offset_min: i32,
    pub variety: Variety,
    /// Mixes the same day come out the same; a refresh changes it.
    pub seed: u64,
    /// Hours between daylist refreshes.
    pub daylist_hours: u32,
}

impl Ctx {
    fn local_ms(&self, at: i64) -> i64 {
        at + self.tz_offset_min as i64 * 60_000
    }
    /// Local hour of a time (0..24).
    pub fn hour(&self, at: i64) -> u32 {
        (self.local_ms(at).rem_euclid(DAY_MS) / 3_600_000) as u32
    }
    /// Local weekday of a time, 0 = Monday.
    pub fn weekday(&self, at: i64) -> u32 {
        // 1970-01-01 was a Thursday.
        ((self.local_ms(at).div_euclid(DAY_MS) + 3).rem_euclid(7)) as u32
    }
    /// The local day a mix is made for: days start at 4 am.
    pub fn mix_day(&self) -> String {
        day_string(self.local_ms(self.now_ms) - 4 * 3_600_000)
    }
    /// The daylist slot: the local day and which block of hours it's in.
    pub fn daylist_slot(&self) -> String {
        let h = self.hour(self.now_ms) / self.daylist_hours.max(1);
        format!("{}#{h}", day_string(self.local_ms(self.now_ms)))
    }
    /// When the next daylist block starts (unix ms).
    pub fn next_daylist_update(&self) -> i64 {
        let hours = self.daylist_hours.max(1) as i64;
        let local = self.local_ms(self.now_ms);
        let day_start = local - local.rem_euclid(DAY_MS);
        let block = (self.hour(self.now_ms) as i64 / hours + 1) * hours;
        day_start + block * 3_600_000 - self.tz_offset_min as i64 * 60_000
    }
}

/// YYYY-MM-DD of a local time in ms.
pub fn day_string(local_ms: i64) -> String {
    // Civil-from-days (Howard Hinnant).
    let z = local_ms.div_euclid(DAY_MS) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// One generated mix.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    pub id: String,
    pub kind: MixKind,
    pub title: String,
    pub description: String,
    pub color: String,
    /// The mix's leading artists (library ids), most present first.
    pub artist_ids: Vec<String>,
    pub tag: Option<String>,
    /// Track indices in play order, with whether each is a favourite.
    pub tracks: Vec<(u32, bool)>,
}

// ------------------------------------------------------------------ helpers

/// splitmix64: a fast, well-mixed hash for deterministic jitter.
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn str_hash(s: &str) -> u64 {
    s.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

/// A number in [0, 1) from a seed and an index.
pub fn jitter(seed: u64, i: u32) -> f32 {
    (mix64(seed ^ (i as u64).wrapping_mul(0x9e37_79b9)) >> 40) as f32 / (1u64 << 24) as f32
}

/// The share of favourites in a mix for a Variety setting.
pub fn familiar_share(v: Variety) -> f32 {
    match v {
        Variety::Familiar => 0.85,
        Variety::Balanced => 0.7,
        Variety::Adventurous => 0.5,
    }
}

type TagVec = HashMap<u16, f32>;

fn normalise(v: &mut TagVec) {
    let n = v.values().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.values_mut().for_each(|x| *x /= n);
    }
}

fn cosine(a: &TagVec, b: &TagVec) -> f32 {
    let (s, l) = if a.len() < b.len() { (a, b) } else { (b, a) };
    s.iter().map(|(k, x)| l.get(k).map_or(0.0, |y| x * y)).sum()
}

fn track_vec(lib: &Lib, t: u32) -> TagVec {
    let mut v: TagVec = lib.tracks[t as usize].tags.iter().map(|g| (*g, 1.0)).collect();
    normalise(&mut v);
    v
}

/// Keeps any artist to `max_run` songs in a row where the list allows it,
/// moving the nearest song by someone else forward.
pub fn space_artists(lib: &Lib, list: &mut [(u32, bool)], max_run: usize) {
    let artist = |e: &(u32, bool)| lib.tracks[e.0 as usize].artist;
    for i in max_run..list.len() {
        let a = artist(&list[i]);
        if (1..=max_run).all(|k| artist(&list[i - k]) == a)
            && let Some(j) = (i + 1..list.len()).find(|&j| artist(&list[j]) != a)
        {
            list[i..=j].rotate_right(1);
        }
    }
}

/// Puts the fresh songs in among the favourites, evenly.
fn interleave(familiar: Vec<u32>, fresh: Vec<u32>) -> Vec<(u32, bool)> {
    let total = familiar.len() + fresh.len();
    let mut out = Vec::with_capacity(total);
    let (mut f, mut n) = (familiar.into_iter().peekable(), fresh.into_iter().peekable());
    let ratio = if total == 0 { 0.0 } else { n.len() as f32 / total as f32 };
    let mut owed = 0.0f32;
    while out.len() < total {
        owed += ratio;
        if (owed >= 1.0 || f.peek().is_none()) && n.peek().is_some() {
            owed -= 1.0;
            out.push((n.next().unwrap(), false));
        } else if let Some(t) = f.next() {
            out.push((t, true));
        } else if let Some(t) = n.next() {
            out.push((t, false));
        }
    }
    out
}

/// Takes from `ranked` in order, skipping used songs and keeping each artist
/// under `per_artist`.
fn take(lib: &Lib, ranked: &[u32], n: usize, per_artist: usize, used: &mut HashSet<u32>, counts: &mut HashMap<u32, usize>) -> Vec<u32> {
    let mut out = Vec::with_capacity(n);
    for &t in ranked {
        if out.len() >= n {
            break;
        }
        let a = lib.tracks[t as usize].artist;
        let c = counts.entry(a).or_default();
        if *c >= per_artist || used.contains(&t) {
            continue;
        }
        *c += 1;
        used.insert(t);
        out.push(t);
    }
    out
}

/// "A, B, C and more" from the mix's leading artists.
fn artists_line(lib: &Lib, artists: &[u32]) -> String {
    let names: Vec<&str> = artists.iter().take(3).map(|a| lib.artists[*a as usize].title.as_str()).filter(|n| !n.is_empty()).collect();
    match names.len() {
        0 => String::new(),
        1 => format!("{} and more", names[0]),
        _ => format!("{} and more", names.join(", ")),
    }
}

/// The artists with most songs in a list, most first.
fn leading_artists(lib: &Lib, tracks: &[(u32, bool)]) -> Vec<u32> {
    let mut count: HashMap<u32, (usize, usize)> = HashMap::new();
    for (i, (t, _)) in tracks.iter().enumerate() {
        let e = count.entry(lib.tracks[*t as usize].artist).or_insert((0, i));
        e.0 += 1;
    }
    let mut v: Vec<_> = count.into_iter().collect();
    v.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.1.1.cmp(&b.1.1)));
    v.into_iter().map(|(a, _)| a).take(6).collect()
}

#[allow(clippy::too_many_arguments)]
fn finish(lib: &Lib, id: String, kind: MixKind, title: String, description: Option<String>, color: usize, tag: Option<String>, mut tracks: Vec<(u32, bool)>) -> Generated {
    space_artists(lib, &mut tracks, 2);
    let lead = leading_artists(lib, &tracks);
    let description = description.unwrap_or_else(|| artists_line(lib, &lead));
    Generated {
        id,
        kind,
        title,
        description,
        color: PALETTE[color % PALETTE.len()].to_string(),
        artist_ids: lead.iter().map(|a| lib.artists[*a as usize].id.clone()).collect(),
        tag,
        tracks,
    }
}

// ------------------------------------------------------------------ taste

/// Per-artist taste: how much you play them and their tag profile.
pub struct Taste {
    pub artist_score: Vec<f32>,
    pub artist_tags: Vec<TagVec>,
    /// Your overall tag profile, weighted by plays.
    pub profile: TagVec,
    /// Plex similar artists of each artist, as library artists.
    similar: Vec<Vec<u32>>,
}

impl Taste {
    pub fn new(lib: &Lib) -> Taste {
        let n = lib.artists.len();
        let mut artist_score = vec![0.0f32; n];
        let mut artist_tags: Vec<TagVec> = vec![HashMap::new(); n];
        let mut profile: TagVec = HashMap::new();
        // Rarer tags tell more about an artist than ones everything has.
        let total = lib.tracks.len().max(1) as f32;
        let idf: Vec<f32> = lib.by_tag.iter().map(|ts| (total / (1.0 + ts.len() as f32)).ln().max(0.1)).collect();
        for (i, t) in lib.tracks.iter().enumerate() {
            let a = t.artist as usize;
            let aff = lib.affinity(i as u32).max(0.0);
            artist_score[a] += aff;
            for g in &t.tags {
                *artist_tags[a].entry(*g).or_default() += idf[*g as usize];
                if aff > 0.0 {
                    *profile.entry(*g).or_default() += aff * idf[*g as usize];
                }
            }
        }
        artist_tags.iter_mut().for_each(normalise);
        normalise(&mut profile);
        let by_title: HashMap<String, u32> =
            lib.artists.iter().enumerate().map(|(i, a)| (a.title.trim().to_lowercase(), i as u32)).collect();
        let similar = lib
            .artists
            .iter()
            .map(|a| a.similar.iter().filter_map(|n| by_title.get(&n.trim().to_lowercase()).copied()).collect())
            .collect();
        Taste { artist_score, artist_tags, profile, similar }
    }

    /// How alike two artists are: shared tags, listened together, and Plex
    /// listing one as similar to the other.
    pub fn similarity(&self, lib: &Lib, a: u32, b: u32) -> f32 {
        if a == b {
            return 1.0;
        }
        let tags = cosine(&self.artist_tags[a as usize], &self.artist_tags[b as usize]);
        let co = lib.co_artists.get(&(a.min(b), a.max(b))).copied().unwrap_or(0.0);
        let plex = self.similar[a as usize].contains(&b) || self.similar[b as usize].contains(&a);
        0.6 * tags + 0.3 * (co / 3.0).min(1.0) + if plex { 0.3 } else { 0.0 }
    }

    /// Played artists, most played first.
    pub fn top_artists(&self, n: usize) -> Vec<u32> {
        let mut v: Vec<u32> = (0..self.artist_score.len() as u32).filter(|a| self.artist_score[*a as usize] > 0.0).collect();
        v.sort_by(|a, b| self.artist_score[*b as usize].total_cmp(&self.artist_score[*a as usize]).then(a.cmp(b)));
        v.truncate(n);
        v
    }
}

// ------------------------------------------------------------------ daily mixes

/// Groups the top artists: each joins the group it's most like, or starts a
/// new one while there are fewer than [`DAILY_MAX`].
pub fn clusters(lib: &Lib, taste: &Taste) -> Vec<Vec<u32>> {
    let mut groups: Vec<Vec<u32>> = Vec::new();
    for a in taste.top_artists(DAILY_ARTISTS) {
        let best = groups
            .iter()
            .enumerate()
            .map(|(i, g)| (i, g.iter().map(|b| taste.similarity(lib, a, *b)).sum::<f32>() / g.len() as f32))
            .max_by(|x, y| x.1.total_cmp(&y.1));
        match best {
            Some((i, s)) if s >= CLUSTER_JOIN || groups.len() >= DAILY_MAX => groups[i].push(a),
            _ => groups.push(vec![a]),
        }
    }
    groups
}

/// Daily Mixes. `reserved` songs are already in another Daily Mix today.
pub fn daily_mixes(lib: &Lib, taste: &Taste, ctx: &Ctx, reserved: &HashSet<u32>) -> Vec<Generated> {
    let mut used: HashSet<u32> = reserved.clone();
    let mut out = Vec::new();
    for group in clusters(lib, taste) {
        let n = out.len() + 1;
        let seed = ctx.seed ^ str_hash(&format!("daily{n}"));
        if let Some(g) = daily_mix(lib, taste, ctx, &group, n, seed, &mut used) {
            out.push(g);
        }
    }
    out
}

fn daily_mix(lib: &Lib, taste: &Taste, ctx: &Ctx, group: &[u32], n: usize, seed: u64, used: &mut HashSet<u32>) -> Option<Generated> {
    let members: HashSet<u32> = group.iter().copied().collect();
    let mut centroid: TagVec = HashMap::new();
    for a in group {
        for (k, v) in &taste.artist_tags[*a as usize] {
            *centroid.entry(*k).or_default() += v;
        }
    }
    normalise(&mut centroid);
    // Close artists outside the group lend a few fresh songs.
    let mut near: Vec<(u32, f32)> = (0..lib.artists.len() as u32)
        .filter(|a| !members.contains(a) && !lib.by_artist[*a as usize].is_empty())
        .map(|a| (a, group.iter().map(|b| taste.similarity(lib, a, *b)).sum::<f32>() / group.len() as f32))
        .filter(|(_, s)| *s >= NEIGHBOUR_MIN)
        .collect();
    near.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    near.truncate(NEIGHBOURS);
    let near: HashSet<u32> = near.into_iter().map(|(a, _)| a).collect();

    let mut fav: Vec<(u32, f32)> = Vec::new();
    let mut fresh: Vec<(u32, f32)> = Vec::new();
    for a in members.iter().chain(near.iter()) {
        let own = members.contains(a);
        for &t in &lib.by_artist[*a as usize] {
            if !lib.tracks[t as usize].eligible {
                continue;
            }
            if own && lib.familiar(t) {
                fav.push((t, lib.affinity(t) + jitter(seed, t)));
            } else if lib.fresh(t) {
                let close = cosine(&track_vec(lib, t), &centroid);
                fresh.push((t, close + if own { 0.3 } else { 0.0 } + 0.5 * jitter(seed, t)));
            }
        }
    }
    let rank = |mut v: Vec<(u32, f32)>| {
        v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        v.into_iter().map(|(t, _)| t).collect::<Vec<_>>()
    };
    let (fav, fresh) = (rank(fav), rank(fresh));
    let want_fav = (DAILY_LEN as f32 * familiar_share(ctx.variety)).round() as usize;
    let per_artist = (DAILY_LEN * 2 / 5).max(DAILY_LEN / group.len().max(1) + 4);
    let mut counts = HashMap::new();
    let mut f = take(lib, &fav, want_fav, per_artist, used, &mut counts);
    let mut nw = take(lib, &fresh, DAILY_LEN - f.len(), per_artist, used, &mut counts);
    // Short on one side: the other fills in.
    if f.len() + nw.len() < DAILY_LEN {
        f.extend(take(lib, &fav, DAILY_LEN - f.len() - nw.len(), per_artist, used, &mut counts));
    }
    if f.len() + nw.len() < DAILY_LEN {
        nw.extend(take(lib, &fresh, DAILY_LEN - f.len() - nw.len(), usize::MAX, used, &mut counts));
    }
    if f.len() + nw.len() < MIX_MIN {
        for t in f.iter().chain(&nw) {
            used.remove(t);
        }
        return None;
    }
    let tracks = interleave(f, nw);
    Some(finish(lib, format!("daily:{n}"), MixKind::Daily, format!("Daily Mix {n}"), None, n - 1, None, tracks))
}

// ------------------------------------------------------------------ the others

/// Discover: hardly played songs closest to your taste, two per artist.
pub fn discover(lib: &Lib, taste: &Taste, ctx: &Ctx) -> Option<Generated> {
    let seed = ctx.seed ^ str_hash("discover");
    let top = taste.top_artists(10);
    let max_score = taste.artist_score.iter().copied().fold(0.0f32, f32::max).max(1e-6);
    // How close each artist is to the ones you play most (worked out once).
    let like: Vec<f32> = (0..lib.artists.len() as u32)
        .map(|a| top.iter().map(|b| taste.similarity(lib, a, *b)).fold(0.0f32, f32::max))
        .collect();
    let mut scored: Vec<(u32, f32)> = (0..lib.tracks.len() as u32)
        .filter(|t| lib.tracks[*t as usize].eligible && lib.tracks[*t as usize].plays == 0 && lib.tracks[*t as usize].skips == 0)
        .map(|t| {
            let a = lib.tracks[t as usize].artist;
            let known = taste.artist_score[a as usize] / max_score;
            let like = like[a as usize];
            let tags = cosine(&track_vec(lib, t), &taste.profile);
            (t, 0.5 * tags + 0.3 * like.max(0.7 * known) + 0.35 * jitter(seed, t))
        })
        .collect();
    if scored.is_empty() {
        return None;
    }
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let ranked: Vec<u32> = scored.into_iter().map(|(t, _)| t).collect();
    let picked = take(lib, &ranked, SHORT_LEN, 2, &mut HashSet::new(), &mut HashMap::new());
    (picked.len() >= MIX_MIN).then(|| {
        let tracks = picked.into_iter().map(|t| (t, false)).collect();
        finish(lib, "discover".into(), MixKind::Discover, "Discover".into(), Some("Songs from your library you haven't played yet, picked for you.".into()), 2, None, tracks)
    })
}

/// New in your library: albums added in the last 30 days, newest first.
pub fn new_in_library(lib: &Lib, ctx: &Ctx) -> Option<Generated> {
    let since = ctx.now_ms / 1000 - NEW_WINDOW_S;
    let mut albums: Vec<u32> = (0..lib.albums.len() as u32).filter(|a| lib.albums[*a as usize].added_at >= since).collect();
    albums.sort_by(|a, b| lib.albums[*b as usize].added_at.cmp(&lib.albums[*a as usize].added_at).then(a.cmp(b)));
    let mut by_album: HashMap<u32, Vec<u32>> = HashMap::new();
    for (i, t) in lib.tracks.iter().enumerate() {
        if t.eligible && lib.albums[t.album as usize].added_at >= since {
            by_album.entry(t.album).or_default().push(i as u32);
        }
    }
    let mut tracks = Vec::new();
    for a in albums {
        let Some(mut ts) = by_album.remove(&a) else { continue };
        // Unplayed songs first, then album order.
        ts.sort_by_key(|t| (lib.tracks[*t as usize].plays > 0, lib.tracks[*t as usize].track_number, *t));
        tracks.extend(ts.into_iter().take(NEW_PER_ALBUM).map(|t| (t, false)));
        if tracks.len() >= DAILY_LEN {
            break;
        }
    }
    tracks.truncate(DAILY_LEN);
    (tracks.len() >= MIX_MIN).then(|| {
        finish(lib, "new".into(), MixKind::NewInLibrary, "New in your library".into(), Some("The latest albums added to your library.".into()), 4, None, tracks)
    })
}

/// On Repeat: most played in the last 30 days (two plays at least).
pub fn on_repeat(lib: &Lib, ctx: &Ctx) -> Option<Generated> {
    let since = ctx.now_ms - ON_REPEAT_WINDOW_MS;
    let mut count: HashMap<u32, (u32, i64)> = HashMap::new();
    for e in lib.plays.iter().filter(|e| !e.skipped && e.at_ms >= since) {
        let c = count.entry(e.track).or_insert((0, 0));
        c.0 += 1;
        c.1 = c.1.max(e.at_ms);
    }
    let mut v: Vec<(u32, (u32, i64))> =
        count.into_iter().filter(|(t, (n, _))| *n >= 2 && lib.tracks[*t as usize].eligible).collect();
    v.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(b.1.1.cmp(&a.1.1)).then(a.0.cmp(&b.0)));
    let tracks: Vec<(u32, bool)> = v.into_iter().take(SHORT_LEN).map(|(t, _)| (t, true)).collect();
    (tracks.len() >= MIX_MIN).then(|| {
        finish(lib, "onRepeat".into(), MixKind::OnRepeat, "On Repeat".into(), Some("Songs you can't stop playing this month.".into()), 0, None, tracks)
    })
}

/// Rediscover: played a lot before, not once in the last six months.
pub fn rediscover(lib: &Lib, ctx: &Ctx) -> Option<Generated> {
    let before = ctx.now_ms - REDISCOVER_AFTER_MS;
    let seed = ctx.seed ^ str_hash("rediscover");
    let mut v: Vec<(u32, f32)> = (0..lib.tracks.len() as u32)
        .filter(|t| {
            let tr = &lib.tracks[*t as usize];
            tr.eligible && tr.plays >= REDISCOVER_MIN_PLAYS && tr.last_played_ms.is_some_and(|l| l < before)
        })
        .map(|t| (t, lib.affinity(t) + 0.5 * jitter(seed, t)))
        .collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let ranked: Vec<u32> = v.into_iter().map(|(t, _)| t).collect();
    let picked = take(lib, &ranked, SHORT_LEN, 4, &mut HashSet::new(), &mut HashMap::new());
    (picked.len() >= MIX_MIN).then(|| {
        let tracks = picked.into_iter().map(|t| (t, true)).collect();
        finish(lib, "rediscover".into(), MixKind::Rediscover, "Rediscover".into(), Some("Old favourites you haven't played in a while.".into()), 6, None, tracks)
    })
}

/// A mix of one tag's songs: favourites first, fresh ones mixed in.
fn tag_mix(lib: &Lib, ctx: &Ctx, tag: u16, kind: MixKind, color: usize, share: f32) -> Option<Generated> {
    let name = lib.tags[tag as usize].clone();
    let seed = ctx.seed ^ str_hash(&format!("tag{name}"));
    let pool: Vec<u32> = lib.by_tag[tag as usize].iter().copied().filter(|t| lib.tracks[*t as usize].eligible).collect();
    let mut fav: Vec<(u32, f32)> = pool.iter().filter(|t| lib.familiar(**t)).map(|t| (*t, lib.affinity(*t) + jitter(seed, *t))).collect();
    let mut fresh: Vec<(u32, f32)> = pool.iter().filter(|t| !lib.familiar(**t) && lib.tracks[**t as usize].skips < 2).map(|t| (*t, jitter(seed, *t))).collect();
    fav.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    fresh.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let fav: Vec<u32> = fav.into_iter().map(|(t, _)| t).collect();
    let fresh: Vec<u32> = fresh.into_iter().map(|(t, _)| t).collect();
    let want_fav = (TAG_MIX_LEN as f32 * share).round() as usize;
    let per_artist = TAG_MIX_LEN / 5;
    let (mut used, mut counts) = (HashSet::new(), HashMap::new());
    let mut f = take(lib, &fav, want_fav, per_artist, &mut used, &mut counts);
    let mut nw = take(lib, &fresh, TAG_MIX_LEN - f.len(), per_artist, &mut used, &mut counts);
    if f.len() + nw.len() < TAG_MIX_LEN {
        f.extend(take(lib, &fav, TAG_MIX_LEN - f.len() - nw.len(), usize::MAX, &mut used, &mut counts));
    }
    if f.len() + nw.len() < TAG_MIX_LEN {
        nw.extend(take(lib, &fresh, TAG_MIX_LEN - f.len() - nw.len(), usize::MAX, &mut used, &mut counts));
    }
    if f.len() + nw.len() < MIX_MIN {
        return None;
    }
    let id = format!("{}:{}", if kind == MixKind::Mood { "mood" } else { "genre" }, name.to_lowercase());
    Some(finish(lib, id, kind, format!("{name} Mix"), None, color, Some(name), interleave(f, nw)))
}

/// Genre (and style) mixes and mood mixes: tags with enough songs, the ones
/// you play most first.
pub fn tag_mixes(lib: &Lib, ctx: &Ctx) -> (Vec<Generated>, Vec<Generated>) {
    let share = familiar_share(ctx.variety);
    let mut ranked: Vec<(u16, f32, usize)> = (0..lib.tags.len() as u16)
        .filter_map(|g| {
            let pool = &lib.by_tag[g as usize];
            let n = pool.iter().filter(|t| lib.tracks[**t as usize].eligible).count();
            (n >= TAG_MIX_MIN).then(|| (g, pool.iter().map(|t| lib.affinity(*t).max(0.0)).sum::<f32>(), n))
        })
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.2.cmp(&a.2)).then(lib.tags[a.0 as usize].cmp(&lib.tags[b.0 as usize])));
    let (mut genres, mut moods) = (Vec::new(), Vec::new());
    for (g, _, _) in ranked {
        let mood = lib.tag_kinds[g as usize] == TagKind::Mood;
        let list = if mood { &mut moods } else { &mut genres };
        if list.len() >= TAG_MIXES_MAX {
            continue;
        }
        let kind = if mood { MixKind::Mood } else { MixKind::Genre };
        if let Some(m) = tag_mix(lib, ctx, g, kind, list.len() + if mood { 3 } else { 7 }, share) {
            list.push(m);
        }
    }
    (genres, moods)
}

// ------------------------------------------------------------------ daylist

/// The part of the day an hour falls in.
pub fn part_of_day(hour: u32) -> &'static str {
    match hour {
        0..=4 => "late night",
        5..=8 => "early morning",
        9..=11 => "morning",
        12..=16 => "afternoon",
        17..=20 => "evening",
        _ => "night",
    }
}

const WEEKDAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

/// The daylist: songs you play around this hour (weekend and weekday apart),
/// newer plays weighing more, with fresh songs of its leading tag mixed in.
pub fn daylist(lib: &Lib, ctx: &Ctx) -> Option<Generated> {
    let hour = ctx.hour(ctx.now_ms) as i32;
    let weekend = ctx.weekday(ctx.now_ms) >= 5;
    let seed = ctx.seed ^ str_hash(&ctx.daylist_slot());
    let mut score: HashMap<u32, f32> = HashMap::new();
    for e in lib.plays.iter().filter(|e| !e.skipped) {
        let h = ctx.hour(e.at_ms) as i32;
        let d = (h - hour).rem_euclid(24).min((hour - h).rem_euclid(24));
        if d > 1 {
            continue;
        }
        let same_kind = (ctx.weekday(e.at_ms) >= 5) == weekend;
        let age_days = ((ctx.now_ms - e.at_ms).max(0) / DAY_MS) as f32;
        let w = if same_kind { 1.0 } else { 0.4 } * if d == 0 { 1.0 } else { 0.6 } * 0.5f32.powf(age_days / 30.0);
        *score.entry(e.track).or_default() += w;
    }
    let mut fav: Vec<(u32, f32)> = score.into_iter().filter(|(t, _)| lib.tracks[*t as usize].eligible).map(|(t, s)| (t, s + 0.1 * jitter(seed, t))).collect();
    fav.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let fav: Vec<u32> = fav.into_iter().map(|(t, _)| t).collect();
    let want_fav = (DAYLIST_LEN as f32 * familiar_share(ctx.variety)).round() as usize;
    let (mut used, mut counts) = (HashSet::new(), HashMap::new());
    let f = take(lib, &fav, want_fav, 4, &mut used, &mut counts);
    // The leading tag of what you play now (or overall, with nothing yet).
    let mut tag_w: HashMap<u16, f32> = HashMap::new();
    let basis: Vec<u32> = if f.is_empty() {
        (0..lib.tracks.len() as u32).filter(|t| lib.familiar(*t)).collect()
    } else {
        f.clone()
    };
    for t in &basis {
        for g in &lib.tracks[*t as usize].tags {
            *tag_w.entry(*g).or_default() += 1.0;
        }
    }
    let lead = tag_w.into_iter().max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.cmp(&a.0))).map(|(g, _)| g);
    let mut fresh: Vec<(u32, f32)> = match lead {
        Some(g) => lib.by_tag[g as usize].iter().copied().filter(|t| lib.tracks[*t as usize].eligible && lib.fresh(*t)).map(|t| (t, jitter(seed, t))).collect(),
        None => Vec::new(),
    };
    fresh.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let fresh: Vec<u32> = fresh.into_iter().map(|(t, _)| t).collect();
    let mut nw = take(lib, &fresh, DAYLIST_LEN - f.len(), 4, &mut used, &mut counts);
    if let (true, Some(g)) = (f.is_empty(), lead) {
        // Nothing played around this hour yet: your favourites of its tag lead.
        let mut favs: Vec<u32> = lib.by_tag[g as usize].iter().copied().filter(|t| lib.tracks[*t as usize].eligible && lib.familiar(*t)).collect();
        favs.sort_by(|a, b| lib.affinity(*b).total_cmp(&lib.affinity(*a)).then(a.cmp(b)));
        let more = take(lib, &favs, DAYLIST_LEN.saturating_sub(nw.len()), 4, &mut used, &mut counts);
        let tracks = interleave(more, std::mem::take(&mut nw));
        return daylist_of(lib, ctx, lead, tracks);
    }
    let tracks = interleave(f, nw);
    daylist_of(lib, ctx, lead, tracks)
}

fn daylist_of(lib: &Lib, ctx: &Ctx, lead: Option<u16>, tracks: Vec<(u32, bool)>) -> Option<Generated> {
    if tracks.len() < MIX_MIN {
        return None;
    }
    let hour = ctx.hour(ctx.now_ms);
    let part = part_of_day(hour);
    let day = WEEKDAYS[ctx.weekday(ctx.now_ms) as usize];
    // Its genres and moods, most present first.
    let mut weight: HashMap<u16, usize> = HashMap::new();
    for (t, _) in &tracks {
        for g in &lib.tracks[*t as usize].tags {
            *weight.entry(*g).or_default() += 1;
        }
    }
    let mut tags: Vec<(u16, usize)> = weight.into_iter().collect();
    tags.sort_by(|a, b| b.1.cmp(&a.1).then(lib.tags[a.0 as usize].cmp(&lib.tags[b.0 as usize])));
    if let Some(g) = lead
        && let Some(i) = tags.iter().position(|(t, _)| *t == g)
    {
        let first = tags.remove(i);
        tags.insert(0, first);
    }
    let names: Vec<String> = tags.iter().map(|(g, _)| lib.tags[*g as usize].to_lowercase()).collect();
    let words = names.iter().take(2).cloned().collect::<Vec<_>>().join(" ");
    let title = [words.as_str(), &day.to_lowercase(), part].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
    let when = match part {
        "late night" => "late at night",
        "early morning" => "in the early morning",
        "morning" => "in the morning",
        "afternoon" => "in the afternoon",
        "evening" => "in the evening",
        _ => "at night",
    };
    let description = if names.is_empty() {
        format!("Inspired by your listening on {day}s {when}.")
    } else {
        format!("Here's some {}, inspired by your listening on {day}s {when}.", names.iter().take(6).cloned().collect::<Vec<_>>().join(", "))
    };
    Some(finish(lib, "daylist".into(), MixKind::Daylist, title, Some(description), 5, lead.map(|g| lib.tags[g as usize].clone()), tracks))
}

/// All-Time Top Songs: most played first, by Plex play count (the listen
/// log fills in plays Plex hasn't heard about yet).
pub fn top_songs(lib: &Lib) -> Option<Generated> {
    let mut v: Vec<u32> = (0..lib.tracks.len() as u32).filter(|t| lib.tracks[*t as usize].eligible && lib.tracks[*t as usize].plays > 0).collect();
    v.sort_by(|a, b| {
        let (x, y) = (&lib.tracks[*a as usize], &lib.tracks[*b as usize]);
        y.plays.cmp(&x.plays).then(y.last_played_ms.cmp(&x.last_played_ms)).then(a.cmp(b))
    });
    v.truncate(TOP_SONGS_LEN);
    // Ranked by plays: no spacing pass here.
    let tracks: Vec<(u32, bool)> = v.into_iter().map(|t| (t, true)).collect();
    if tracks.len() < MIX_MIN {
        return None;
    }
    let lead = leading_artists(lib, &tracks);
    Some(Generated {
        id: "topSongs".into(),
        kind: MixKind::TopSongs,
        title: "All-Time Top Songs".into(),
        description: "Your most played songs, ever.".into(),
        color: PALETTE[8].to_string(),
        artist_ids: lead.iter().map(|a| lib.artists[*a as usize].id.clone()).collect(),
        tag: None,
        tracks,
    })
}

/// Everything for one day, in shelf order.
pub fn generate_all(lib: &Lib, ctx: &Ctx) -> Vec<Generated> {
    let taste = Taste::new(lib);
    let mut out = daily_mixes(lib, &taste, ctx, &HashSet::new());
    out.extend(discover(lib, &taste, ctx));
    out.extend(new_in_library(lib, ctx));
    out.extend(on_repeat(lib, ctx));
    out.extend(rediscover(lib, ctx));
    out.extend(top_songs(lib));
    out.extend(daylist(lib, ctx));
    let (genres, moods) = tag_mixes(lib, ctx);
    out.extend(moods);
    out.extend(genres);
    out
}

/// One mix again with another seed (a Refresh). Daily Mixes keep clear of
/// the other Daily Mixes' songs.
pub fn regenerate_one(lib: &Lib, ctx: &Ctx, id: &str, other_daily: &HashSet<u32>) -> Option<Generated> {
    let taste = Taste::new(lib);
    if let Some(n) = id.strip_prefix("daily:").and_then(|n| n.parse::<usize>().ok()) {
        let group = clusters(lib, &taste).into_iter().nth(n.checked_sub(1)?)?;
        let mut used = other_daily.clone();
        return daily_mix(lib, &taste, ctx, &group, n, ctx.seed ^ str_hash(&format!("daily{n}")), &mut used);
    }
    match id {
        "discover" => discover(lib, &taste, ctx),
        "new" => new_in_library(lib, ctx),
        "onRepeat" => on_repeat(lib, ctx),
        "rediscover" => rediscover(lib, ctx),
        "daylist" => daylist(lib, ctx),
        "topSongs" => top_songs(lib),
        _ => {
            let (genres, moods) = tag_mixes(lib, ctx);
            genres.into_iter().chain(moods).find(|m| m.id == id)
        }
    }
}
