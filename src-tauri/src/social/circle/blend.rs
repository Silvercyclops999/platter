//! Blends: a taste summary made on each computer ([`taste`]), and the
//! playlist each person's Platter makes from everyone's summaries and the
//! songs that person can play ([`generate`]).
//!
//! Only summaries travel. The playlist is about [`TARGET`] songs, made
//! again each day, taking turns between members so each gets about an
//! equal share, and every song says whose taste it came from.

use std::collections::{HashMap, HashSet};

use super::wire::{Ident, Taste, WeightedArtist, WeightedSong};
use super::library::Play;
use crate::db::merge::norm;

/// Songs in a Blend.
pub const TARGET: usize = 50;
/// People in one Blend, you included.
pub const MAX_MEMBERS: usize = 10;
/// The weeks a summary looks back over.
pub const WEEKS: i64 = 8;
pub const TOP_ARTISTS: usize = 15;
pub const TOP_SONGS: usize = 30;
pub const FAVOURITES: usize = 10;
/// Songs taken from one top artist when filling up.
pub const PER_ARTIST: usize = 6;

const DAY: i64 = 86_400;

/// How much a play counts: in full for a week, then half as much for every
/// two weeks older.
fn weight(age_s: i64) -> f64 {
    let days = (age_s.max(0) / DAY) as f64;
    if days <= 7.0 { 1.0 } else { 0.5f64.powf((days - 7.0) / 14.0) }
}

fn per_mille(w: f64, total: f64) -> u32 {
    if total <= 0.0 { 0 } else { ((w / total) * 1000.0).round().clamp(1.0, 1000.0) as u32 }
}

/// A taste summary from this computer's plays (the last [`WEEKS`]) and
/// liked songs. Unreleased songs stay out unless `unreleased`.
pub fn taste(plays: &[Play], who: &HashMap<String, Ident>, favourites: &[Ident], now_s: i64, unreleased: bool) -> Taste {
    let since = now_s - WEEKS * 7 * DAY;
    let mut songs: HashMap<String, (Ident, f64)> = HashMap::new();
    let mut artists: HashMap<String, (String, f64)> = HashMap::new();
    let mut total = 0.0;
    for p in plays.iter().filter(|p| p.at >= since && p.at <= now_s) {
        let Some(s) = who.get(&p.track_id) else { continue };
        if s.u && !unreleased {
            continue;
        }
        let w = weight(now_s - p.at);
        total += w;
        songs.entry(s.key()).or_insert_with(|| (s.clone(), 0.0)).1 += w;
        let a = norm(&s.a);
        if !a.is_empty() {
            artists.entry(a).or_insert_with(|| (s.a.clone(), 0.0)).1 += w;
        }
    }
    let mut so: Vec<(Ident, f64)> = songs.into_values().collect();
    so.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.key().cmp(&b.0.key())));
    let mut ar: Vec<(String, f64)> = artists.into_values().collect();
    ar.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Taste {
        ar: ar.into_iter().take(TOP_ARTISTS).map(|(n, w)| WeightedArtist { n, mb: None, w: per_mille(w, total) }).collect(),
        so: so.into_iter().take(TOP_SONGS).map(|(s, w)| WeightedSong { s, w: per_mille(w, total) }).collect(),
        fav: favourites.iter().filter(|f| unreleased || !f.u).take(FAVOURITES).cloned().collect(),
        at: now_s,
    }
}

/// A small, fixed shuffle (the same `seed`, the same order).
fn mix(seed: u64, i: u64) -> u64 {
    let mut x = seed ^ i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x
}

/// The day number a Blend is made for (its seed).
pub fn day_of(now_s: i64) -> i64 {
    now_s.div_euclid(DAY)
}

/// What one member's taste offers, in the order it's tried.
enum Cand {
    Song(Ident),
    /// An artist's songs here.
    Artist(String),
}

fn candidates(t: &Taste, seed: u64) -> Vec<Cand> {
    // Heaviest first, with neighbours swapped about a bit each day.
    let mut songs: Vec<(u64, &WeightedSong)> = t.so.iter().enumerate().map(|(i, s)| ((i as u64 / 4) << 32 | (mix(seed, i as u64) & 0xffff_ffff), s)).collect();
    songs.sort_by_key(|(k, _)| *k);
    let mut out = Vec::new();
    let mut favs = t.fav.iter();
    for (i, (_, s)) in songs.iter().enumerate() {
        out.push(Cand::Song(s.s.clone()));
        if i % 2 == 1
            && let Some(f) = favs.next()
        {
            out.push(Cand::Song(f.clone()));
        }
    }
    out.extend(favs.map(|f| Cand::Song(f.clone())));
    out.extend(t.ar.iter().map(|a| Cand::Artist(a.n.clone())));
    out
}

/// One song in a made Blend and whose taste it came from (a person id).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Row {
    pub id: String,
    pub by: String,
}

/// A member's song this library doesn't have.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Missing {
    pub s: Ident,
    pub by: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Made {
    pub rows: Vec<Row>,
    pub missing: Vec<Missing>,
}

/// Make a Blend: members take turns (from a day's own starting point), each
/// turn adding that member's next song this library can play and that
/// isn't in yet or hidden. A member with nothing left drops out of the
/// turns. `find` gives the copy here of a song; `artist` an artist's songs
/// here. Unreleased songs stay out unless `unreleased`.
pub fn generate(
    members: &[(String, &Taste)],
    find: &mut dyn FnMut(&Ident) -> Option<String>,
    artist: &mut dyn FnMut(&str) -> Vec<String>,
    hidden: &HashSet<String>,
    unreleased: bool,
    seed: u64,
    me: &str,
) -> Made {
    let mut order: Vec<&(String, &Taste)> = members.iter().collect();
    order.sort_by(|a, b| a.0.cmp(&b.0));
    if !order.is_empty() {
        let r = (mix(seed, 7) % order.len() as u64) as usize;
        order.rotate_left(r);
    }
    let mut queues: Vec<(String, std::collections::VecDeque<Cand>)> = order.iter().map(|(id, t)| (id.clone(), candidates(t, seed ^ mix(seed, id.len() as u64)).into())).collect();
    let mut pending: Vec<std::collections::VecDeque<String>> = vec![Default::default(); queues.len()];
    let mut taken: HashSet<String> = HashSet::new();
    let mut made = Made::default();
    let mut missing_keys: HashSet<String> = HashSet::new();
    let mut found: HashMap<String, Option<String>> = HashMap::new();
    while made.rows.len() < TARGET {
        let mut any = false;
        for (qi, (by, q)) in queues.iter_mut().enumerate() {
            if made.rows.len() >= TARGET {
                break;
            }
            // This member's next song that plays here.
            let pick = loop {
                if let Some(id) = pending[qi].pop_front() {
                    if !taken.contains(&id) && !hidden.contains(&id) {
                        break Some(id);
                    }
                    continue;
                }
                match q.pop_front() {
                    None => break None,
                    Some(Cand::Artist(a)) => pending[qi].extend(artist(&a).into_iter().take(PER_ARTIST)),
                    Some(Cand::Song(s)) => {
                        if s.u && !unreleased {
                            continue;
                        }
                        let key = s.key();
                        let here = found.entry(key.clone()).or_insert_with(|| find(&s)).clone();
                        match here {
                            Some(id) if !taken.contains(&id) && !hidden.contains(&id) => break Some(id),
                            Some(_) => {}
                            None => {
                                if by != me && !s.u && missing_keys.insert(key) {
                                    made.missing.push(Missing { s, by: by.clone() });
                                }
                            }
                        }
                    }
                }
            };
            if let Some(id) = pick {
                taken.insert(id.clone());
                made.rows.push(Row { id, by: by.clone() });
                any = true;
            }
        }
        if !any {
            break;
        }
    }
    made
}

/// "A + B + C".
pub fn title(names: &[String]) -> String {
    names.join(" + ")
}

/// "A Blend of music for A and B" (or "A, B, and C").
pub fn description(names: &[String]) -> String {
    let list = match names {
        [] => String::new(),
        [a] => a.clone(),
        [a, b] => format!("{a} and {b}"),
        [rest @ .., last] => format!("{}, and {last}", rest.join(", ")),
    };
    format!("A Blend of music for {list}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: &str, a: &str) -> Ident {
        Ident { n: n.into(), a: a.into(), d: 200_000, ..Default::default() }
    }

    fn song_taste(songs: &[Ident]) -> Taste {
        Taste { so: songs.iter().map(|s| WeightedSong { s: s.clone(), w: 10 }).collect(), ..Default::default() }
    }

    #[test]
    fn a_summary_weighs_recent_plays_and_leaves_out_unreleased() {
        let now = 100 * DAY;
        let mut who = HashMap::new();
        who.insert("a:1".to_string(), id("Old", "Band A"));
        who.insert("a:2".to_string(), id("New", "Band B"));
        who.insert("a:3".to_string(), Ident { u: true, ..id("Vault", "Band B") });
        let plays = vec![
            Play { track_id: "a:1".into(), at: now - 40 * DAY },
            Play { track_id: "a:1".into(), at: now - 40 * DAY },
            Play { track_id: "a:1".into(), at: now - 41 * DAY },
            Play { track_id: "a:2".into(), at: now - DAY },
            Play { track_id: "a:2".into(), at: now - 2 * DAY },
            Play { track_id: "a:3".into(), at: now - DAY },
            // Too old to count.
            Play { track_id: "a:1".into(), at: now - 80 * DAY },
        ];
        let favs = vec![id("Liked", "Band C"), Ident { u: true, ..id("Liked Demo", "Band C") }];
        let t = taste(&plays, &who, &favs, now, false);
        assert_eq!(t.so[0].s.n, "New", "two plays this week beat three a month ago");
        assert_eq!(t.so.len(), 2);
        assert_eq!(t.ar.iter().map(|a| a.n.as_str()).collect::<Vec<_>>(), vec!["Band B", "Band A"]);
        assert!(t.so.iter().all(|s| !s.s.u) && t.fav.len() == 1);
        let all = taste(&plays, &who, &favs, now, true);
        assert!(all.so.iter().any(|s| s.s.u) && all.fav.len() == 2);
    }

    #[test]
    fn members_take_turns_and_only_playable_songs_go_in() {
        let mine: Vec<Ident> = (0..40).map(|i| id(&format!("Mine {i}"), "Me")).collect();
        let theirs: Vec<Ident> = (0..60).map(|i| id(&format!("Theirs {i}"), "Them")).collect();
        let (a, b) = (song_taste(&mine), song_taste(&theirs));
        // Only every other one of their songs is here.
        let mut find = |s: &Ident| {
            let n: usize = s.n.rsplit(' ').next().unwrap().parse().unwrap();
            (s.a == "Me" || n.is_multiple_of(2)).then(|| format!("x:{}", s.n))
        };
        let mut artist = |_: &str| Vec::new();
        let made = generate(&[("me".into(), &a), ("them".into(), &b)], &mut find, &mut artist, &HashSet::new(), false, 3, "me");
        assert_eq!(made.rows.len(), TARGET);
        let mine_n = made.rows.iter().filter(|r| r.by == "me").count();
        assert_eq!(mine_n, 25, "an equal share each");
        // Turns alternate.
        assert!(made.rows.windows(2).all(|w| w[0].by != w[1].by));
        // Their songs here are theirs; the rest are missing, never mine.
        assert!(made.rows.iter().filter(|r| r.by == "them").all(|r| r.id.starts_with("x:Theirs")));
        assert!(!made.missing.is_empty() && made.missing.iter().all(|m| m.by == "them"));
    }

    #[test]
    fn a_member_with_little_overlap_drops_out_and_others_fill_up() {
        let mine: Vec<Ident> = (0..60).map(|i| id(&format!("Mine {i}"), "Me")).collect();
        let theirs: Vec<Ident> = (0..60).map(|i| id(&format!("Theirs {i}"), "Them")).collect();
        let (a, b) = (song_taste(&mine), song_taste(&theirs));
        let mut find = |s: &Ident| (s.a == "Me" || s.n == "Theirs 0" || s.n == "Theirs 1").then(|| format!("x:{}", s.n));
        let mut artist = |_: &str| Vec::new();
        let made = generate(&[("me".into(), &a), ("them".into(), &b)], &mut find, &mut artist, &HashSet::new(), false, 1, "me");
        assert_eq!(made.rows.iter().filter(|r| r.by == "them").count(), 2);
        assert_eq!(made.rows.len(), TARGET);
        assert_eq!(made.missing.len(), 58);
    }

    #[test]
    fn hidden_and_unreleased_songs_stay_out_and_artists_fill_in() {
        let t = Taste {
            so: vec![WeightedSong { s: id("Hide Me", "A"), w: 9 }, WeightedSong { s: Ident { u: true, ..id("Vault", "A") }, w: 8 }],
            ar: vec![WeightedArtist { n: "A".into(), mb: None, w: 5 }],
            ..Default::default()
        };
        let mut find = |s: &Ident| Some(format!("x:{}", s.n));
        let mut artist = |a: &str| vec![format!("x:{a} deep cut"), "x:Hide Me".into()];
        let hidden: HashSet<String> = ["x:Hide Me".to_string()].into();
        let made = generate(&[("me".into(), &t)], &mut find, &mut artist, &hidden, false, 0, "me");
        assert_eq!(made.rows, vec![Row { id: "x:A deep cut".into(), by: "me".into() }]);
        let with = generate(&[("me".into(), &t)], &mut find, &mut artist, &hidden, true, 0, "me");
        assert!(with.rows.iter().any(|r| r.id == "x:Vault"));
    }

    #[test]
    fn the_same_day_makes_the_same_blend() {
        let songs: Vec<Ident> = (0..30).map(|i| id(&format!("S {i}"), "A")).collect();
        let t = song_taste(&songs);
        let run = |seed| {
            let mut find = |s: &Ident| Some(s.n.clone());
            let mut artist = |_: &str| Vec::new();
            generate(&[("me".into(), &t)], &mut find, &mut artist, &HashSet::new(), false, seed, "me").rows
        };
        assert_eq!(run(5), run(5));
        assert_ne!(run(5), run(6));
    }

    #[test]
    fn titles_and_descriptions() {
        let n = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(title(&n(&["Robin", "Jules"])), "Robin + Jules");
        assert_eq!(description(&n(&["Robin", "Jules"])), "A Blend of music for Robin and Jules");
        assert_eq!(description(&n(&["Robin", "Jules", "Casey"])), "A Blend of music for Robin, Jules, and Casey");
    }
}
