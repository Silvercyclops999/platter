//! Checking lyrics against the recording (pure parts).
//!
//! A short window of the song is transcribed on this computer, and the words
//! heard are compared with each candidate's words. The comparison is a local
//! alignment of folded words (case, accents, punctuation and filler sounds
//! taken out): a run of words in the same order scores, scattered shared
//! words ("the", "king") barely do. A chorus sung more often than the lyrics
//! write it, or ad-libs between lines, don't count against a match: the
//! alignment is taken again on what is left over.
//!
//! The verdicts:
//! - Match: the words line up; synced lyrics are in time.
//! - Match with offset: they line up but the timing is off by a steady
//!   amount (measured, rounded to 0.1 s).
//! - No match: enough was heard and it isn't these lyrics.
//! - Can't tell: too little was heard (an instrumental part, a quiet
//!   window), or the lyrics aren't in a language the check can read.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::transcribe::lines::{Segment, to_lines};
use crate::plex::details::Lyrics;

/// How much of the song is transcribed for a check.
pub const WINDOW_MS: u32 = 30_000;
/// Fewer words than this heard in a window: can't tell.
pub const MIN_HEARD: usize = 6;
/// At or above: the words match.
pub const MATCH_SCORE: f32 = 0.5;
/// Below: they don't (when enough was heard).
pub const NO_MATCH_SCORE: f32 = 0.25;
/// A measured offset smaller than this is left alone.
pub const OFFSET_MIN_MS: i32 = 300;
/// Offsets at the two windows further apart than this are drift.
pub const DRIFT_MIN_MS: i32 = 400;
/// Synced lyrics are first compared within this much of the window.
const NEAR_MS: u32 = 60_000;
/// A window quieter than this (RMS) isn't transcribed: can't tell.
pub const QUIET_RMS: f32 = 0.004;

/// Sounds that carry no words.
const FILLERS: &[&str] = &[
    "oh", "ooh", "ohh", "ah", "ahh", "uh", "yeah", "yea", "ya", "hey", "woah", "whoa", "mm", "mmm", "hmm", "huh", "la", "na",
    "da", "ay", "ayy", "yo", "uhh", "eh",
];

/// Common English words: lyrics with too few of them aren't in a language
/// the check (an English speech model) can judge.
const ENGLISH: &[&str] = &[
    "the", "and", "you", "to", "my", "it", "in", "is", "of", "your", "on", "that", "we", "for", "be", "all", "so", "but",
    "with", "this", "know", "what", "just", "like", "dont", "cant", "im", "its", "when", "got", "get", "they", "was", "she",
    "he", "her", "his", "can", "never", "now", "up", "out", "if", "baby", "love", "go", "are", "do", "at", "from", "i",
];

/// Short words that say little about which song it is.
const STOP: &[&str] = &[
    "the", "a", "an", "and", "or", "to", "of", "in", "on", "at", "i", "im", "me", "my", "you", "your", "it", "its", "is",
    "be", "we", "us", "so", "but", "for", "with", "that", "this", "all", "up", "no", "do", "dont",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Match,
    MatchOffset,
    NoMatch,
    CantTell,
}

impl Verdict {
    pub fn matched(self) -> bool {
        matches!(self, Verdict::Match | Verdict::MatchOffset)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Match => "match",
            Verdict::MatchOffset => "matchOffset",
            Verdict::NoMatch => "noMatch",
            Verdict::CantTell => "cantTell",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "match" => Verdict::Match,
            "matchOffset" => Verdict::MatchOffset,
            "noMatch" => Verdict::NoMatch,
            "cantTell" => Verdict::CantTell,
            _ => return None,
        })
    }
}

/// One word heard in the recording, at its time in the song.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeardWord {
    #[serde(rename = "w")]
    pub text: String,
    #[serde(rename = "t")]
    pub ms: u32,
}

/// One word of the lyrics; `ms` is its time when the lyrics are synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LyricWord {
    pub text: String,
    pub ms: Option<u32>,
    /// Its time is written down (a timed word, or the first word of a timed
    /// line), not spread over the line: offsets are measured on these.
    pub anchor: bool,
}

/// What a check found for one candidate.
#[derive(Debug, Clone, PartialEq)]
pub struct Judged {
    pub verdict: Verdict,
    pub score: f32,
    /// Heard minus written, in ms (positive: the lyrics are early).
    pub offset_ms: i32,
    /// Where in the lyrics the offset was measured.
    pub at_ms: Option<u32>,
}

impl Judged {
    fn cant_tell(score: f32) -> Self {
        Self { verdict: Verdict::CantTell, score, offset_ms: 0, at_ms: None }
    }
}

/// A word for comparing: lower case, no accents or punctuation, "-ing"
/// sung as "-in'" made the same. None for filler sounds and empty words.
pub fn fold_word(w: &str) -> Option<String> {
    let mut s: String =
        w.nfkd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect();
    if s.is_empty() || FILLERS.contains(&s.as_str()) {
        return None;
    }
    if s.len() >= 5 && s.ends_with("ing") {
        s.pop();
    }
    Some(s)
}

/// Words of a text, folded, in order.
pub fn fold_text(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || c == '-' || c == '/').filter_map(fold_word).collect()
}

/// What the recognizer heard in a window starting at `start_ms`, as timed
/// folded words (sound descriptions and stock phrases dropped).
pub fn heard_words(segments: &[Segment], start_ms: u32) -> Vec<HeardWord> {
    let mut out = Vec::new();
    for line in to_lines(segments) {
        for w in &line.words {
            for t in fold_text(&w.text) {
                out.push(HeardWord { text: t, ms: start_ms.saturating_add(w.start_ms) });
            }
        }
    }
    out
}

/// The lyrics' words. Synced lines without word times spread their words
/// evenly over the line.
pub fn lyric_words(l: &Lyrics) -> Vec<LyricWord> {
    let mut out = Vec::new();
    if l.synced && !l.timed.is_empty() {
        let lines: Vec<_> = l.timed.iter().filter(|t| !t.text.trim().is_empty()).collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.words.is_empty() {
                for w in &line.words {
                    for (k, t) in fold_text(&w.text).into_iter().enumerate() {
                        out.push(LyricWord { text: t, ms: Some(w.start_ms), anchor: k == 0 });
                    }
                }
                continue;
            }
            let words = fold_text(&line.text);
            let next = lines.get(i + 1).map(|n| n.start_ms).unwrap_or(line.start_ms + 4000);
            let span = next.saturating_sub(line.start_ms).min(6000);
            let n = words.len().max(1) as u32;
            for (k, t) in words.into_iter().enumerate() {
                out.push(LyricWord { text: t, ms: Some(line.start_ms + span * k as u32 / n), anchor: k == 0 });
            }
        }
    } else {
        for line in &l.lines {
            for t in fold_text(line) {
                out.push(LyricWord { text: t, ms: None, anchor: false });
            }
        }
    }
    out
}

/// Whether two folded words are the same word as sung (one letter off is
/// allowed in longer words: the recognizer mishears).
fn same(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (la, lb) = (a.chars().count(), b.chars().count());
    if la.min(lb) < 4 || la.abs_diff(lb) > 1 {
        return false;
    }
    one_edit(a, b)
}

fn one_edit(a: &str, b: &str) -> bool {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (mut i, mut j, mut edits) = (0, 0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        match a.len().cmp(&b.len()) {
            std::cmp::Ordering::Greater => i += 1,
            std::cmp::Ordering::Less => j += 1,
            std::cmp::Ordering::Equal => {
                i += 1;
                j += 1;
            }
        }
    }
    edits + (a.len() - i) + (b.len() - j) <= 1
}

const HIT: i32 = 2;
const MISS: i32 = -1;
const GAP: i32 = -1;

/// One local alignment (Smith-Waterman) of heard words against lyric words:
/// its score, the heard range it covers, and the matched pairs.
fn local(heard: &[&str], lyric: &[&str]) -> (i32, usize, usize, Vec<(usize, usize)>) {
    let (n, m) = (heard.len(), lyric.len());
    if n == 0 || m == 0 {
        return (0, 0, 0, Vec::new());
    }
    let w = m + 1;
    let mut h = vec![0i32; (n + 1) * w];
    // 0 stop, 1 diagonal, 2 up (skip a heard word), 3 left (skip a lyric word)
    let mut dir = vec![0u8; (n + 1) * w];
    let (mut best, mut bi, mut bj) = (0, 0, 0);
    for i in 1..=n {
        for j in 1..=m {
            let hit = same(heard[i - 1], lyric[j - 1]);
            let d = h[(i - 1) * w + j - 1] + if hit { HIT } else { MISS };
            let u = h[(i - 1) * w + j] + GAP;
            let l = h[i * w + j - 1] + GAP;
            let (v, k) = [(0, 0u8), (d, 1), (u, 2), (l, 3)].into_iter().fold((0, 0u8), |a, b| if b.0 > a.0 { b } else { a });
            h[i * w + j] = v;
            dir[i * w + j] = k;
            if v > best {
                (best, bi, bj) = (v, i, j);
            }
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (bi, bj);
    let end = bi;
    while i > 0 && j > 0 && h[i * w + j] > 0 {
        match dir[i * w + j] {
            1 => {
                if same(heard[i - 1], lyric[j - 1]) {
                    pairs.push((i - 1, j - 1));
                }
                i -= 1;
                j -= 1;
            }
            2 => i -= 1,
            3 => j -= 1,
            _ => break,
        }
    }
    pairs.reverse();
    (best, i, end, pairs)
}

/// Local alignments taken again on the heard words left over on each side
/// (repeats, ad-libs), summed. Returns the total and every matched pair.
fn align(heard: &[&str], lyric: &[&str]) -> (i32, Vec<(usize, usize)>) {
    fn go(heard: &[&str], lyric: &[&str], base: usize, depth: u8, total: &mut i32, pairs: &mut Vec<(usize, usize)>) {
        if heard.len() < 3 || depth > 6 {
            return;
        }
        let (score, from, to, p) = local(heard, lyric);
        if score < 4 {
            return;
        }
        *total += score;
        pairs.extend(p.into_iter().map(|(i, j)| (i + base, j)));
        go(&heard[..from], lyric, base, depth + 1, total, pairs);
        go(&heard[to..], lyric, base + to, depth + 1, total, pairs);
    }
    let (mut total, mut pairs) = (0, Vec::new());
    go(heard, lyric, 0, 0, &mut total, &mut pairs);
    pairs.sort_unstable();
    (total, pairs)
}

/// Share of the heard words found, in order, in runs of the lyrics: 0 to 1.
/// The alignment only counts runs (two words in a row at least), so words
/// two songs merely share don't add up.
pub fn score(heard: &[HeardWord], lyric: &[LyricWord]) -> (f32, Vec<(usize, usize)>) {
    if heard.is_empty() {
        return (0.0, Vec::new());
    }
    let h: Vec<&str> = heard.iter().map(|w| w.text.as_str()).collect();
    let l: Vec<&str> = lyric.iter().map(|w| w.text.as_str()).collect();
    let (_, pairs) = align(&h, &l);
    ((pairs.len() as f32 / h.len() as f32).clamp(0.0, 1.0), pairs)
}

/// Whether the lyrics are mostly English words (the speech model's
/// language); others can't be judged by what it heard.
pub fn looks_english(l: &Lyrics) -> bool {
    let text = if l.synced && !l.timed.is_empty() {
        l.timed.iter().map(|t| t.text.as_str()).collect::<Vec<_>>().join(" ")
    } else {
        l.lines.join(" ")
    };
    let letters: Vec<char> = text.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return true;
    }
    let ascii = letters.iter().filter(|c| c.is_ascii()).count() as f32 / letters.len() as f32;
    if ascii < 0.8 {
        return false;
    }
    let words = fold_text(&text);
    if words.is_empty() {
        return true;
    }
    let common = words.iter().filter(|w| ENGLISH.contains(&w.as_str())).count() as f32;
    common / words.len() as f32 >= 0.12
}

fn median(mut v: Vec<i64>) -> Option<i64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

/// Rounded to the nearest 0.1 s.
pub fn round_100(ms: i64) -> i32 {
    ((ms as f64 / 100.0).round() * 100.0) as i32
}

/// Compare what was heard in a window with one candidate's lyrics.
pub fn judge(heard: &[HeardWord], lyrics: &Lyrics, window: (u32, u32)) -> Judged {
    if heard.len() < MIN_HEARD {
        return Judged::cant_tell(0.0);
    }
    let words = lyric_words(lyrics);
    if words.is_empty() {
        return Judged::cant_tell(0.0);
    }
    let synced = words.iter().all(|w| w.ms.is_some());
    // Synced lyrics: first the part near the window, so a chorus sung
    // several times lines up with the right one.
    let near: Vec<LyricWord> = if synced {
        let (a, b) = (window.0.saturating_sub(NEAR_MS), window.1.saturating_add(NEAR_MS));
        words.iter().filter(|w| w.ms.is_some_and(|t| t >= a && t <= b)).cloned().collect()
    } else {
        Vec::new()
    };
    let (mut s, mut pairs, mut used) = (0.0, Vec::new(), &words);
    if near.len() >= MIN_HEARD {
        (s, pairs) = score(heard, &near);
        used = &near;
    }
    if s < MATCH_SCORE {
        let (s2, p2) = score(heard, &words);
        if s2 > s {
            (s, pairs, used) = (s2, p2, &words);
        }
    }
    if s >= MATCH_SCORE {
        let mut j = Judged { verdict: Verdict::Match, score: s, offset_ms: 0, at_ms: None };
        if synced {
            // Measured on words whose time is written down, when enough are matched.
            let anchored: Vec<(usize, usize)> = pairs.iter().copied().filter(|&(_, l)| used[l].anchor).collect();
            let on = if anchored.len() >= 2 { &anchored } else { &pairs };
            let diffs: Vec<i64> = on.iter().filter_map(|&(h, l)| Some(heard[h].ms as i64 - used[l].ms? as i64)).collect();
            let at: Vec<i64> = on.iter().filter_map(|&(_, l)| used[l].ms.map(i64::from)).collect();
            if let (Some(off), Some(at)) = (median(diffs), median(at)) {
                j.offset_ms = round_100(off);
                j.at_ms = Some(at.max(0) as u32);
                if j.offset_ms.abs() >= OFFSET_MIN_MS {
                    j.verdict = Verdict::MatchOffset;
                }
            }
        }
        return j;
    }
    if s < NO_MATCH_SCORE && looks_english(lyrics) {
        return Judged { verdict: Verdict::NoMatch, score: s, offset_ms: 0, at_ms: None };
    }
    Judged::cant_tell(s)
}

/// The first window: from just before the first sung line (skipping an
/// instrumental intro), or a little way in for lyrics without timing.
pub fn first_window(first_line_ms: Option<u32>, duration_ms: u32) -> (u32, u32) {
    let start = match first_line_ms {
        Some(t) => t.saturating_sub(2000),
        None => (duration_ms / 100 * 15).max(20_000),
    };
    let start = start.min(duration_ms.saturating_sub(WINDOW_MS));
    (start, (start + WINDOW_MS).min(duration_ms))
}

/// The second window for drift: near the last sung line, well after the
/// first. None when the song is too short for two.
pub fn second_window(first: (u32, u32), last_line_ms: Option<u32>, duration_ms: u32) -> Option<(u32, u32)> {
    let end = last_line_ms.map(|t| t.saturating_add(5000)).unwrap_or(duration_ms.saturating_sub(10_000)).min(duration_ms);
    let start = end.saturating_sub(WINDOW_MS);
    (start >= first.1 + 30_000).then_some((start, end))
}

/// The first and last sung line times of synced lyrics.
pub fn sung_span(l: &Lyrics) -> Option<(u32, u32)> {
    if !l.synced {
        return None;
    }
    let mut sung = l.timed.iter().filter(|t| !t.text.trim().is_empty()).map(|t| t.start_ms);
    let first = sung.next()?;
    Some((first, sung.next_back().unwrap_or(first)))
}

/// A measured offset at a point in the lyrics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub at_ms: u32,
    pub offset_ms: i32,
}

/// The offset to apply at a lyric time: one point is a steady shift; two
/// are stretched linearly between them (and carried on past them).
pub fn offset_at(points: &[Point], t: u32) -> i32 {
    match points {
        [] => 0,
        [p] => p.offset_ms,
        [a, b, ..] => {
            if a.at_ms == b.at_ms {
                return a.offset_ms;
            }
            let slope = (b.offset_ms - a.offset_ms) as f64 / (b.at_ms as f64 - a.at_ms as f64);
            round_100((a.offset_ms as f64 + slope * (t as f64 - a.at_ms as f64)) as i64)
        }
    }
}

/// Move synced lyrics: the measured offsets, then `extra_ms` (the user's nudge).
pub fn retime(l: &mut Lyrics, points: &[Point], extra_ms: i32) {
    if points.is_empty() && extra_ms == 0 {
        return;
    }
    let shift = |t: u32| (t as i64 + offset_at(points, t) as i64 + extra_ms as i64).max(0) as u32;
    for line in &mut l.timed {
        line.start_ms = shift(line.start_ms);
        for w in &mut line.words {
            w.start_ms = shift(w.start_ms);
        }
    }
}

/// Content words of lyrics (no short common words), for telling texts apart.
fn content(l: &Lyrics) -> std::collections::HashSet<String> {
    lyric_words(l).into_iter().map(|w| w.text).filter(|w| w.len() > 2 && !STOP.contains(&w.as_str())).collect()
}

/// Two candidates' texts differ a lot (different songs, not two
/// transcriptions of one).
pub fn disagree(a: &Lyrics, b: &Lyrics) -> bool {
    let (x, y) = (content(a), content(b));
    if x.is_empty() || y.is_empty() {
        return false;
    }
    let both = x.intersection(&y).count() as f32;
    let either = x.union(&y).count() as f32;
    both / either < 0.35
}

/// The words and timing of lyrics, hashed: the check cache's text key.
pub fn lyrics_hash(l: &Lyrics) -> String {
    let mut h = Sha256::new();
    if l.synced && !l.timed.is_empty() {
        h.update(b"s");
        for t in &l.timed {
            h.update(t.start_ms.to_le_bytes());
            h.update(t.text.as_bytes());
            h.update(b"\n");
        }
    } else {
        h.update(b"p");
        for line in &l.lines {
            h.update(line.as_bytes());
            h.update(b"\n");
        }
    }
    h.finalize().iter().take(8).map(|b| format!("{b:02x}")).collect()
}

/// RMS level of samples.
pub fn rms(pcm: &[f32]) -> f32 {
    if pcm.is_empty() {
        return 0.0;
    }
    (pcm.iter().map(|s| s * s).sum::<f32>() / pcm.len() as f32).sqrt()
}
