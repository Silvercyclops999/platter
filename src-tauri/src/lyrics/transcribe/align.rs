//! Lyrics found without timing, lined up to the singing: the real words, at
//! the times the speech model heard them.
//!
//! The lyric's words and the heard words are folded the same way as the
//! lyrics check does (case, accents, punctuation and filler sounds out) and
//! aligned in order. A run of words in a row counts for more than scattered
//! ones, so a common word heard somewhere else doesn't pull a line out of
//! place. Ad-libs and repeats the lyrics don't write are skipped for free;
//! lyric words nobody was heard singing cost a little. A matched word takes
//! its heard time, the words around it are spread between, and a line with
//! nothing matched is placed between its neighbours. Too little matched
//! means these lyrics aren't of this recording: nothing is timed.

use crate::lyrics::check::fold_text;

use super::lines::{Line, Segment, Word, to_lines};

/// Share of the lyric's words that must be heard, in order, to time it.
pub const ENOUGH: f32 = 0.5;
/// A word's length when there's nothing to go by.
const WORD_MS: u32 = 300;
/// The most words either side is aligned over (a very long piece).
const MOST_WORDS: usize = 1500;

/// A folded word and where it came from.
struct Token {
    text: String,
    /// Lyric: (line, word); heard: the word's index in `heard`.
    at: (usize, usize),
}

/// The plain lines timed, or None when they don't fit what was heard.
pub fn time_plain(plain: &[String], heard: &[Segment]) -> Option<Vec<Line>> {
    let lines: Vec<Vec<String>> =
        plain.iter().map(|l| l.split_whitespace().map(str::to_owned).collect::<Vec<_>>()).filter(|w| !w.is_empty()).collect();
    let heard_words: Vec<Word> = to_lines(heard).into_iter().flat_map(|l| l.words).collect();
    let lyric: Vec<Token> = lines
        .iter()
        .enumerate()
        .flat_map(|(li, ws)| ws.iter().enumerate().flat_map(move |(wi, w)| fold_text(w).into_iter().map(move |t| Token { text: t, at: (li, wi) })))
        .collect();
    let sung: Vec<Token> =
        heard_words.iter().enumerate().flat_map(|(i, w)| fold_text(&w.text).into_iter().map(move |t| Token { text: t, at: (i, 0) })).collect();
    if lyric.is_empty() || sung.is_empty() || lyric.len() > MOST_WORDS || sung.len() > MOST_WORDS {
        return None;
    }
    let pairs = align(&lyric, &sung);
    let share = pairs.len() as f32 / lyric.len() as f32;
    if share < ENOUGH {
        tracing::info!(share, "the lyrics found don't fit what was heard: not timed");
        return None;
    }
    // Each lyric word's heard word (its first matched part).
    let mut at: Vec<Vec<Option<(u32, u32)>>> = lines.iter().map(|ws| vec![None; ws.len()]).collect();
    for (l, s) in pairs {
        let (li, wi) = lyric[l].at;
        let w = &heard_words[sung[s].at.0];
        at[li][wi].get_or_insert((w.start_ms, w.end_ms.max(w.start_ms + 1)));
    }
    Some(place(&lines, &at))
}

/// The best in-order pairing of lyric and heard tokens, as index pairs.
fn align(lyric: &[Token], sung: &[Token]) -> Vec<(usize, usize)> {
    const RUN: i32 = 3;
    const ALONE: i32 = 1;
    const UNSUNG: i32 = -1;
    let (n, m) = (lyric.len(), sung.len());
    // Two scores per cell: the best ending on a match, and the best overall.
    // Moves: 0 none, 1 match after a match, 2 match after anything, 3 skip
    // a heard word, 4 skip a lyric word.
    let w = m + 1;
    let mut best = vec![0i32; (n + 1) * w];
    let mut matched = vec![i32::MIN / 2; (n + 1) * w];
    let mut how = vec![0u8; (n + 1) * w];
    let mut how_matched = vec![0u8; (n + 1) * w];
    for i in 1..=n {
        best[i * w] = best[(i - 1) * w] + UNSUNG;
        how[i * w] = 4;
    }
    how[1..=m].fill(3);
    for i in 1..=n {
        for j in 1..=m {
            let c = i * w + j;
            let d = (i - 1) * w + j - 1;
            if lyric[i - 1].text == sung[j - 1].text {
                let (after_run, after_any) = (matched[d] + RUN, best[d] + ALONE);
                (matched[c], how_matched[c]) = if after_run >= after_any { (after_run, 1) } else { (after_any, 2) };
            }
            let skip_heard = best[c - 1];
            let skip_lyric = best[c - w] + UNSUNG;
            (best[c], how[c]) = if matched[c] >= skip_heard && matched[c] >= skip_lyric {
                (matched[c], how_matched[c])
            } else if skip_heard >= skip_lyric {
                (skip_heard, 3)
            } else {
                (skip_lyric, 4)
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (n, m);
    // On a run the next step back must be a match too.
    let mut in_run = false;
    while i > 0 && j > 0 {
        let c = i * w + j;
        let step = if in_run { how_matched[c] } else { how[c] };
        match step {
            1 | 2 => {
                pairs.push((i - 1, j - 1));
                in_run = step == 1;
                i -= 1;
                j -= 1;
            }
            3 => j -= 1,
            4 => i -= 1,
            _ => break,
        }
    }
    pairs.reverse();
    pairs
}

/// Times for every word: heard ones as heard, the rest spread between.
fn place(lines: &[Vec<String>], at: &[Vec<Option<(u32, u32)>>]) -> Vec<Line> {
    // Each line's first and last heard times, where it has any.
    let known: Vec<Option<(u32, u32)>> = at
        .iter()
        .map(|ws| {
            let first = ws.iter().flatten().next()?;
            let last = ws.iter().flatten().last()?;
            Some((first.0, last.1))
        })
        .collect();
    let mut out: Vec<Line> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if known[i].is_some() {
            out.push(line_with(&lines[i], &at[i], out.last().map_or(0, |l: &Line| l.end_ms)));
            i += 1;
            continue;
        }
        // A run of lines with nothing heard: share the time between the
        // line before and the line after by their words.
        let from = out.last().map_or(0, |l| l.end_ms);
        let mut k = i;
        while k < lines.len() && known[k].is_none() {
            k += 1;
        }
        let total: usize = lines[i..k].iter().map(Vec::len).sum();
        let until = known.get(k).copied().flatten().map_or(from + total as u32 * WORD_MS, |(start, _)| start);
        let span = until.saturating_sub(from);
        let mut t = from;
        for ws in &lines[i..k] {
            let len = span * ws.len() as u32 / total.max(1) as u32;
            out.push(spread(ws, t, t + len));
            t += len;
        }
        i = k;
    }
    out
}

/// A line with some heard words: theirs as heard, the others in between
/// (or a word's length either side at the ends).
fn line_with(words: &[String], at: &[Option<(u32, u32)>], not_before: u32) -> Line {
    let mut times: Vec<(u32, u32)> = Vec::with_capacity(words.len());
    for (k, t) in at.iter().enumerate() {
        if let Some(t) = t {
            times.push(*t);
            continue;
        }
        let prev = times.last().map(|p| p.1);
        let next = at[k..].iter().enumerate().find_map(|(d, t)| t.map(|t| (d, t.0)));
        let (start, end) = match (prev, next) {
            (Some(p), Some((d, n))) => {
                let step = n.saturating_sub(p) / (d as u32 + 1);
                (p, p + step.max(1))
            }
            (Some(p), None) => (p, p + WORD_MS),
            (None, Some((d, n))) => {
                let start = n.saturating_sub(WORD_MS * d as u32).max(not_before);
                (start, (start + WORD_MS).min(n).max(start + 1))
            }
            (None, None) => (not_before, not_before + WORD_MS),
        };
        times.push((start, end));
    }
    // Never back in time.
    for k in 1..times.len() {
        if times[k].0 < times[k - 1].0 {
            times[k].0 = times[k - 1].0;
        }
        times[k].1 = times[k].1.max(times[k].0 + 1);
    }
    let words: Vec<Word> = words.iter().zip(&times).map(|(w, t)| Word { start_ms: t.0, end_ms: t.1, text: w.clone() }).collect();
    Line { start_ms: times[0].0, end_ms: times.last().map_or(times[0].1, |t| t.1), words }
}

/// A line nobody was heard singing, its words spread evenly over a span.
fn spread(words: &[String], from: u32, to: u32) -> Line {
    let n = words.len().max(1) as u32;
    let step = (to.saturating_sub(from) / n).max(1);
    let words: Vec<Word> =
        words.iter().enumerate().map(|(k, w)| Word { start_ms: from + step * k as u32, end_ms: from + step * (k as u32 + 1), text: w.clone() }).collect();
    Line { start_ms: from, end_ms: from + step * n, words }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Heard words, one segment, `gap` ms apart from `at`.
    fn heard(text: &str, at: u32) -> Segment {
        Segment {
            words: text
                .split(' ')
                .enumerate()
                .map(|(i, w)| Word { start_ms: at + i as u32 * 400, end_ms: at + i as u32 * 400 + 300, text: format!(" {w}") })
                .collect(),
            sure: None,
        }
    }

    fn plain(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_real_words_take_the_heard_times() {
        let lyrics = plain(&["Hello darkness, my old friend", "", "I've come to talk with you again"]);
        // Misheard words and an ad-lib between the lines.
        let sung = vec![heard("hello darkness my oh friend", 10_000), heard("yeah", 13_000), heard("i've come to talk with you again", 15_000)];
        let lines = time_plain(&lyrics, &sung).unwrap();
        assert_eq!(lines.len(), 2, "the blank line goes");
        assert_eq!(lines[0].text(), "Hello darkness, my old friend", "the lyric's own words");
        assert_eq!(lines[0].start_ms, 10_000);
        assert_eq!(lines[0].words[1].start_ms, 10_400);
        // "old" wasn't heard: between "my" and "friend".
        assert!(lines[0].words[3].start_ms > lines[0].words[2].start_ms && lines[0].words[3].start_ms < lines[0].words[4].start_ms);
        assert_eq!(lines[1].start_ms, 15_000);
    }

    #[test]
    fn a_line_nobody_was_heard_singing_goes_between_its_neighbours() {
        let lyrics = plain(&["one two three four", "mumbled words here", "five six seven eight"]);
        let sung = vec![heard("one two three four", 1_000), heard("five six seven eight", 9_000)];
        let lines = time_plain(&lyrics, &sung).unwrap();
        assert_eq!(lines.len(), 3);
        assert!(lines[1].start_ms >= lines[0].end_ms && lines[1].end_ms <= 9_000, "{:?}", (lines[1].start_ms, lines[1].end_ms));
        assert_eq!(lines[2].start_ms, 9_000);
    }

    #[test]
    fn a_chorus_sung_twice_is_placed_twice() {
        let lyrics = plain(&["verse words go here now", "we are the chorus line", "second verse words now", "we are the chorus line"]);
        let sung = vec![
            heard("verse words go here now", 0),
            heard("we are the chorus line", 5_000),
            heard("second verse words now", 10_000),
            heard("we are the chorus line", 15_000),
        ];
        let lines = time_plain(&lyrics, &sung).unwrap();
        let starts: Vec<u32> = lines.iter().map(|l| l.start_ms).collect();
        assert_eq!(starts, vec![0, 5_000, 10_000, 15_000]);
    }

    #[test]
    fn lyrics_of_another_song_are_not_timed() {
        let lyrics = plain(&["completely different words entirely", "nothing like the recording at all"]);
        let sung = vec![heard("hello darkness my old friend", 0), heard("i've come to talk with you again", 4_000)];
        assert!(time_plain(&lyrics, &sung).is_none());
        assert!(time_plain(&[], &sung).is_none());
        assert!(time_plain(&lyrics, &[]).is_none());
    }

    #[test]
    fn a_word_heard_elsewhere_doesnt_pull_a_line_out_of_place() {
        // "love" alone is heard early; the line is sung later in a run.
        let lyrics = plain(&["intro words are here", "all you need is love"]);
        let sung = vec![heard("love", 500), heard("intro words are here", 2_000), heard("all you need is love", 8_000)];
        let lines = time_plain(&lyrics, &sung).unwrap();
        assert_eq!(lines[1].start_ms, 8_000);
        assert_eq!(lines[1].words[4].start_ms, 9_600);
    }

    #[test]
    fn times_never_go_back() {
        let lyrics = plain(&["red blue green gold", "pink grey black white"]);
        let sung = vec![heard("red blue green gold pink grey black white", 1_000)];
        let lines = time_plain(&lyrics, &sung).unwrap();
        let all: Vec<u32> = lines.iter().flat_map(|l| l.words.iter().map(|w| w.start_ms)).collect();
        assert!(all.windows(2).all(|p| p[0] <= p[1]), "{all:?}");
    }
}
