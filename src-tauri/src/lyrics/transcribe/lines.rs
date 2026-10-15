//! Recognized words to lyric lines, and lines to enhanced LRC text (the
//! stored form, which the lyrics parser and the editor both read).
//!
//! Speech recognition hands back segments of timed words. A new line starts
//! at each segment, at a pause in the singing, and when a line runs long.
//! Sound descriptions ("[Music]", "(upbeat music)") and music symbols are
//! dropped, as are the stock phrases the recognizer invents over silence.

/// One recognized word. `text` keeps its leading space when the recognizer
/// gave one; punctuation is already attached.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Word {
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
}

/// Words the recognizer grouped together (roughly a phrase or sentence).
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub words: Vec<Word>,
}

/// One lyric line: its words, trimmed and joined with single spaces.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub start_ms: u32,
    pub end_ms: u32,
    pub words: Vec<Word>,
}

impl Line {
    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// A silence this long ends a line.
pub const PAUSE_MS: u32 = 700;
/// A shorter silence ends a line that already has this many words.
const SOFT_PAUSE_MS: u32 = 250;
const SOFT_WORDS: usize = 8;
/// No line gets longer than this.
const MAX_WORDS: usize = 14;
/// A gap this long between lines gets an instrumental marker.
pub const GAP_MS: u32 = 6000;

/// Whole segments the recognizer makes up over music or silence.
const STOCK_PHRASES: [&str; 6] = [
    "thank you for watching",
    "thanks for watching",
    "please subscribe",
    "subtitles by",
    "you",
    "thank you",
];

/// Words in parentheses that describe a sound rather than a lyric.
const SOUND_WORDS: [&str; 12] = [
    "music", "instrumental", "applause", "laugh", "laughs", "laughing", "sigh", "silence", "inaudible", "noise",
    "beat", "humming",
];

fn is_music_symbol(c: char) -> bool {
    matches!(c, '♪' | '♫' | '♬' | '♩' | '🎵' | '🎶')
}

fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Drops sound descriptions, music symbols and empty words; trims each word.
fn clean_words(words: &[Word]) -> Vec<Word> {
    let mut out = Vec::new();
    // Inside a description: the closing character that ends it.
    let mut skip_until: Option<char> = None;
    // A parenthesis group being held until we know whether it's a sound.
    let mut held: Vec<Word> = Vec::new();
    for w in words {
        let text: String = w.text.chars().filter(|c| !is_music_symbol(*c)).collect();
        let text = text.trim();
        if let Some(close) = skip_until {
            if text.contains(close) {
                skip_until = None;
            }
            continue;
        }
        if text.is_empty() {
            continue;
        }
        if text.starts_with('[') || text.starts_with('*') {
            let close = if text.starts_with('[') { ']' } else { '*' };
            if !text[1..].contains(close) {
                skip_until = Some(close);
            }
            continue;
        }
        let word = Word { text: text.to_owned(), ..w.clone() };
        if !held.is_empty() || text.starts_with('(') {
            held.push(word);
            if text.contains(')') {
                let all: String = held.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
                let inner = fold(&all);
                let sound = inner.split(' ').any(|t| SOUND_WORDS.contains(&t));
                if !sound {
                    out.append(&mut held);
                }
                held.clear();
            }
            continue;
        }
        out.push(word);
    }
    // An unclosed parenthesis: keep what was sung.
    out.append(&mut held);
    out
}

fn is_stock(words: &[Word]) -> bool {
    let all: String = words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
    let f = fold(&all);
    f.is_empty() || STOCK_PHRASES.contains(&f.as_str())
}

/// First letter up, trailing comma or full stop off (lyrics don't end lines with them).
fn tidy_line(mut words: Vec<Word>) -> Vec<Word> {
    if let Some(first) = words.first_mut() {
        let mut cs = first.text.chars();
        if let Some(c) = cs.next()
            && c.is_lowercase()
        {
            first.text = c.to_uppercase().chain(cs).collect();
        }
    }
    if let Some(last) = words.last_mut() {
        let t = last.text.trim_end_matches([',', ';', ':']);
        let t = if t.ends_with('.') && !t.ends_with("..") { &t[..t.len() - 1] } else { t };
        last.text = t.to_owned();
    }
    words.retain(|w| !w.text.is_empty());
    words
}

/// Segments to lines: a new line per segment, on a pause, or when long.
pub fn to_lines(segments: &[Segment]) -> Vec<Line> {
    let mut lines = Vec::new();
    for seg in segments {
        let words = clean_words(&seg.words);
        if is_stock(&words) {
            continue;
        }
        let mut cur: Vec<Word> = Vec::new();
        for w in words {
            if let Some(prev) = cur.last() {
                let gap = w.start_ms.saturating_sub(prev.end_ms);
                let split = gap >= PAUSE_MS || (gap >= SOFT_PAUSE_MS && cur.len() >= SOFT_WORDS) || cur.len() >= MAX_WORDS;
                if split {
                    push_line(&mut lines, std::mem::take(&mut cur));
                }
            }
            cur.push(w);
        }
        push_line(&mut lines, cur);
    }
    lines
}

fn push_line(lines: &mut Vec<Line>, words: Vec<Word>) {
    let words = tidy_line(words);
    let (Some(first), Some(last)) = (words.first(), words.last()) else { return };
    let (start_ms, end_ms) = (first.start_ms, last.end_ms.max(first.start_ms));
    lines.push(Line { start_ms, end_ms, words });
}

/// `mm:ss.xx`.
fn stamp(ms: u32) -> String {
    let cs = ms / 10;
    format!("{:02}:{:02}.{:02}", cs / 6000, (cs / 100) % 60, cs % 100)
}

/// Lines as enhanced LRC: a line time, then a time before every word. A long
/// silence between lines gets an empty timed line (shown as a gap).
pub fn to_lrc(lines: &[Line]) -> String {
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            let prev = &lines[i - 1];
            if l.start_ms.saturating_sub(prev.end_ms) >= GAP_MS {
                out.push_str(&format!("[{}]\n", stamp(prev.end_ms + 500)));
            }
        }
        out.push_str(&format!("[{}]", stamp(l.start_ms)));
        for (j, w) in l.words.iter().enumerate() {
            if j > 0 {
                out.push(' ');
            }
            out.push_str(&format!("<{}>{}", stamp(w.start_ms), w.text));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::details::parse_lyrics_text;

    fn w(start: u32, end: u32, text: &str) -> Word {
        Word { start_ms: start, end_ms: end, text: text.into() }
    }

    fn seg(words: Vec<Word>) -> Segment {
        Segment { words }
    }

    #[test]
    fn splits_on_segments_and_pauses() {
        let s = vec![
            seg(vec![w(1000, 1300, " the"), w(1300, 1700, " river"), w(1700, 2000, " runs,"), w(3000, 3300, " under"), w(3300, 3600, " bridges.")]),
            seg(vec![w(4000, 4400, " I"), w(4400, 4800, " count")]),
        ];
        let lines = to_lines(&s);
        let texts: Vec<String> = lines.iter().map(Line::text).collect();
        assert_eq!(texts, vec!["The river runs", "Under bridges", "I count"]);
        assert_eq!(lines[1].start_ms, 3000);
    }

    #[test]
    fn long_lines_split_at_short_pauses_and_at_the_cap() {
        let words: Vec<Word> = (0..20u32).map(|i| w(i * 400, i * 400 + 300, " la")).collect();
        let lines = to_lines(&[seg(words)]);
        // A 100 ms gap after every word: under the soft pause, so the cap splits.
        assert_eq!(lines[0].words.len(), MAX_WORDS);
        assert_eq!(lines.len(), 2);
        let words: Vec<Word> = (0..12u32).map(|i| w(i * 600, i * 600 + 300, " oh")).collect();
        let lines = to_lines(&[seg(words)]);
        // 300 ms gaps: a line ends once it has the soft count.
        assert_eq!(lines[0].words.len(), SOFT_WORDS);
    }

    #[test]
    fn sound_descriptions_and_symbols_are_dropped_but_backing_vocals_stay() {
        let s = vec![
            seg(vec![w(0, 500, " [Music]")]),
            seg(vec![w(500, 800, " ♪"), w(800, 1000, " (upbeat"), w(1000, 1200, " music)"), w(1200, 1500, " hold"), w(1500, 1800, " on")]),
            seg(vec![w(2000, 2200, " (oh"), w(2200, 2400, " yeah)"), w(2400, 2600, " [inaudible"), w(2600, 2800, " words]")]),
            seg(vec![w(3000, 3200, " ♪"), w(3200, 3400, " Thank"), w(3400, 3600, " you"), w(3600, 3800, " for"), w(3800, 4000, " watching!")]),
        ];
        let texts: Vec<String> = to_lines(&s).iter().map(Line::text).collect();
        assert_eq!(texts, vec!["Hold on", "(oh yeah)"]);
    }

    #[test]
    fn lrc_round_trips_through_the_lyrics_parser() {
        let s = vec![
            seg(vec![w(12_340, 12_800, " hello"), w(12_800, 13_200, " world")]),
            seg(vec![w(25_000, 25_400, " again")]),
        ];
        let lrc = to_lrc(&to_lines(&s));
        assert_eq!(lrc, "[00:12.34]<00:12.34>Hello <00:12.80>world\n[00:13.70]\n[00:25.00]<00:25.00>Again\n");
        let l = parse_lyrics_text(&lrc);
        assert!(l.synced);
        assert_eq!(l.timed.len(), 3);
        assert_eq!(l.timed[0].text, "Hello world");
        assert_eq!(l.timed[0].words.len(), 2);
        assert_eq!(l.timed[0].words[1].start_ms, 12_800);
        assert_eq!(l.timed[1].text, "", "a long silence is a gap line");
        assert_eq!(l.timed[2].start_ms, 25_000);
    }

    #[test]
    fn nothing_sung_gives_no_lines() {
        assert!(to_lines(&[seg(vec![w(0, 100, " [BLANK_AUDIO]")]), seg(vec![w(200, 300, " you")])]).is_empty());
        assert_eq!(to_lrc(&[]), "");
    }
}
