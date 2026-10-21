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

/// Words the recognizer grouped together (roughly a phrase or sentence),
/// and how sure it was of them on average (0 to 1) when it said.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Segment {
    pub words: Vec<Word>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sure: Option<f32>,
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

/// A segment the recognizer was less sure of than this is left out: over
/// music, made-up lines come out around 0.2 to 0.4, sung ones well above.
pub const UNSURE: f32 = 0.4;
/// When more than this share of the words is unsure, the song gets no
/// lyrics rather than the few lines left.
const MOSTLY_UNSURE: f32 = 0.6;

/// Words in parentheses that describe a sound rather than a lyric.
const SOUND_WORDS: [&str; 12] = [
    "music", "instrumental", "applause", "laugh", "laughs", "laughing", "sigh", "silence", "inaudible", "noise",
    "beat", "humming",
];

/// Music notes the recognizer writes over singing or music, and the
/// replacement character older builds left where a note was split between
/// tokens.
fn is_music_symbol(c: char) -> bool {
    matches!(c, '♪' | '♫' | '♬' | '♩' | '🎵' | '🎶' | '\u{fffd}')
}

/// Stored transcription text without music symbols (saved before they were
/// dropped as the lines were made).
pub fn strip_music_symbols(text: &str) -> String {
    text.lines()
        .map(|l| {
            let t: String = l.chars().filter(|c| !is_music_symbol(*c)).collect();
            if t.len() == l.len() { t } else { t.split(' ').filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ") }
        })
        .collect::<Vec<_>>()
        .join("\n")
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

/// The segments worth showing: unsure ones and the prompt read back (the
/// title and artist the model was given, written out over music) are left
/// out, and a song that's mostly unsure gets none at all.
pub fn keep_sure(segments: Vec<Segment>, prompt: &str) -> Vec<Segment> {
    let prompt = fold(prompt);
    let echo = |seg: &Segment| {
        let all: String = seg.words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        !prompt.is_empty() && fold(&all) == prompt
    };
    let total: usize = segments.iter().map(|s| s.words.len()).sum();
    let (kept, dropped): (Vec<Segment>, Vec<Segment>) =
        segments.into_iter().partition(|s| s.sure.is_none_or(|p| p >= UNSURE) && !echo(s));
    let unsure: usize = dropped.iter().map(|s| s.words.len()).sum();
    if total > 0 && unsure as f32 / total as f32 > MOSTLY_UNSURE {
        tracing::info!(words = total, unsure, "transcription mostly unsure: no lyrics");
        return Vec::new();
    }
    if unsure > 0 {
        tracing::info!(words = total, unsure, "unsure lines left out");
    }
    kept
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

/// `mm:ss.xx` to ms.
fn parse_stamp(s: &str) -> Option<u32> {
    let (m, rest) = s.split_once(':')?;
    let (sec, cs) = rest.split_once('.').unwrap_or((rest, "0"));
    let cs: u32 = format!("{:0<2}", &cs[..cs.len().min(2)]).parse().ok()?;
    Some(m.parse::<u32>().ok()? * 60_000 + sec.parse::<u32>().ok()? * 1000 + cs * 10)
}

/// One stored line with word times: its words as (time, text).
fn timed_words(line: &str) -> Option<Vec<(u32, String)>> {
    let rest = line.strip_prefix('[')?;
    let (_, rest) = rest.split_once(']')?;
    if !rest.starts_with('<') {
        return None;
    }
    let mut out = Vec::new();
    for part in rest.split('<').skip(1) {
        let (t, text) = part.split_once('>')?;
        out.push((parse_stamp(t)?, text.trim_end().to_owned()));
    }
    (!out.is_empty()).then_some(out)
}

/// A transcription stored with the first timing (each word at its first
/// token's attention time, which is where that token ends) moved to the
/// current rule without hearing the song again: a word starts about one
/// token earlier, never before the word before it. Words stand in for
/// tokens (most sung words are one token). Gap lines and lines without word
/// times stay. None when there's nothing to move.
pub fn retime_v1(lrc: &str) -> Option<String> {
    let rows: Vec<Option<Vec<(u32, String)>>> = lrc.lines().map(timed_words).collect();
    let old: Vec<u32> = rows.iter().flatten().flatten().map(|w| w.0).collect();
    if old.is_empty() {
        return None;
    }
    let mut new = Vec::with_capacity(old.len());
    for (i, &at) in old.iter().enumerate() {
        let next = old.get(i + 1).copied().filter(|n| *n > at && n - at <= 2000);
        let len = next.map_or(250, |n| (n - at).clamp(80, 600));
        let prev = if i > 0 { old[i - 1].min(at) } else { 0 };
        new.push(at.saturating_sub((len as f32 * 0.75).round() as u32).max(prev));
    }
    let mut k = 0;
    let mut out = String::new();
    for (line, row) in lrc.lines().zip(&rows) {
        match row {
            Some(words) => {
                out.push_str(&format!("[{}]", stamp(new[k])));
                for (j, (_, text)) in words.iter().enumerate() {
                    if j > 0 {
                        out.push(' ');
                    }
                    out.push_str(&format!("<{}>{}", stamp(new[k]), text));
                    k += 1;
                }
            }
            None => out.push_str(line),
        }
        out.push('\n');
    }
    (out.trim_end() != lrc.trim_end()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::details::parse_lyrics_text;

    fn w(start: u32, end: u32, text: &str) -> Word {
        Word { start_ms: start, end_ms: end, text: text.into() }
    }

    fn seg(words: Vec<Word>) -> Segment {
        Segment { words, sure: None }
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
    fn music_notes_and_broken_characters_are_dropped() {
        let s = vec![
            seg(vec![w(0, 300, " ♪"), w(300, 600, " I'm"), w(600, 900, " fine♪"), w(900, 1000, " ♫")]),
            // Only notes (music playing): no line at all.
            seg(vec![w(5000, 5300, " ♪"), w(5300, 5600, " \u{fffd}\u{fffd}"), w(5600, 5900, " ♬")]),
            seg(vec![w(9000, 9300, " \u{fffd}\u{fffd}"), w(9300, 9600, " still"), w(9600, 9900, " here")]),
        ];
        let texts: Vec<String> = to_lines(&s).iter().map(Line::text).collect();
        assert_eq!(texts, vec!["I'm fine", "Still here"]);
    }

    #[test]
    fn stored_text_from_older_builds_loses_its_notes() {
        let old = "[00:01.00]\u{fffd}\u{fffd} I'm fine \u{fffd}\u{fffd}\n[00:04.00]<00:04.00> ♪ <00:05.00> \n[00:07.00]Plain";
        assert_eq!(strip_music_symbols(old), "[00:01.00] I'm fine\n[00:04.00]<00:04.00> <00:05.00>\n[00:07.00]Plain");
        let l = parse_lyrics_text(&strip_music_symbols(old));
        assert!(l.lines.iter().all(|t| !t.contains('\u{fffd}') && !t.contains('♪')), "{:?}", l.lines);
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

    fn sure(p: f32, words: &[&str]) -> Segment {
        Segment { words: words.iter().enumerate().map(|(i, t)| w(i as u32 * 300, i as u32 * 300 + 200, t)).collect(), sure: Some(p) }
    }

    #[test]
    fn unsure_lines_are_left_out() {
        let segs = vec![sure(0.9, &[" Hold", " on"]), sure(0.2, &[" banana", " river"]), sure(0.8, &[" to", " me", " now"])];
        let kept = keep_sure(segs, "");
        let text: Vec<String> = kept.iter().map(|s| s.words.iter().map(|w| w.text.trim()).collect::<Vec<_>>().join(" ")).collect();
        assert_eq!(text, vec!["Hold on", "to me now"]);
        assert_eq!(keep_sure(vec![seg(vec![w(0, 100, " hi")])], "").len(), 1, "no score keeps the line");
    }

    #[test]
    fn a_mostly_unsure_song_gets_no_lyrics() {
        let segs = vec![sure(0.9, &[" Hey"]), sure(0.3, &[" la", " la", " la"]), sure(0.1, &[" na", " na"])];
        assert!(keep_sure(segs, "").is_empty());
    }

    #[test]
    fn the_title_read_back_is_left_out() {
        let segs = vec![sure(0.95, &[" Golden", " Hour", " by", " Kacey."]), sure(0.9, &[" Golden", " hour", " shining"])];
        let kept = keep_sure(segs, "Golden Hour by Kacey.");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].words[2].text, " shining", "a sung line with the title stays");
    }
}
