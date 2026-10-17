//! Folding names so the same song spelled two ways compares equal.
//!
//! Every rule lives here, for every feature that asks "is this the same
//! song": case and accents fold away, "&" reads as "and", a leading "The"
//! drops from artist names, featured artists and producer credits move out
//! of artist and title, and version notes ("(Demo)", "- Snippet", "[v2]",
//! "(Remastered 2011)", "(Live at ...)") split off into their own field so a
//! demo can be told apart from the released song without losing the link.
//!
//! What never merges: "Pt. 1" and "Pt. 2", "Song" and "Song (Interlude)",
//! and brackets the rules don't recognise as a version note (those stay in
//! the title, so an unknown suffix means a different song, never a wrong
//! match). Bare generic titles ("Intro", "Skit") are only the same song on
//! the same album; see [`is_generic`].

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Whether "Part II" and "Part 2" are the same title. Off until decided:
/// the roman numeral stays as written, so they differ.
pub const ROMAN_PART_EQUALS_NUMBER: bool = false;

/// Album artist names that say nothing about who made the song.
const NO_ARTIST: &[&str] = &["various artists", "various", "va", "unknown artist", "unknown"];

/// Bare titles too common to mean the same song across albums.
const GENERIC: &[&str] = &["intro", "outro", "interlude", "skit", "untitled"];

/// Words that make a bracket or dash part a version of the song (it splits
/// off into `version_fold`). Anything else stays in the title.
const VERSION_WORDS: &[&str] = &[
    "demo", "snippet", "live", "remaster", "remastered", "mastered", "unmastered", "remix", "rmx", "mix", "edit",
    "version", "acoustic", "instrumental", "acapella", "cappella", "unplugged", "session", "sessions", "rehearsal",
    "leak", "leaked", "og", "alternate", "alt", "extended", "radio", "clean", "explicit", "dirty", "mono", "stereo",
    "deluxe", "bonus", "rework", "reworked", "cdq", "hq", "lq", "unfinished", "early", "preview", "slowed", "sped",
    "reverb", "chopped", "screwed", "vip", "dub", "outtake", "stripped", "rough", "mixdown", "reference", "edition",
    "expanded", "anniversary", "take",
];

/// Words of a version note that leave the recording the same: a remaster,
/// an edition or a content label. A note made only of these (plus numbers
/// and filler words) doesn't make another version.
const SAME_RECORDING_WORDS: &[&str] = &[
    "remaster", "remastered", "mastered", "deluxe", "bonus", "track", "explicit", "clean", "dirty", "mono", "stereo",
    "edition", "expanded", "anniversary", "album", "single", "lp", "original", "version", "mix", "main", "digital",
    "the", "of", "from", "and", "year", "th", "st", "nd", "rd",
];

/// Version words that always mean another recording, even next to a
/// remaster note ("Remastered Demo" is still a demo).
const STRONG_VERSION_WORDS: &[&str] = &[
    "demo", "snippet", "live", "remix", "rmx", "edit", "acoustic", "instrumental", "acapella", "cappella", "unplugged",
    "session", "sessions", "rehearsal", "leak", "leaked", "og", "alternate", "alt", "extended", "radio", "rework",
    "reworked", "cdq", "hq", "lq", "unfinished", "early", "preview", "slowed", "sped", "reverb", "chopped", "screwed",
    "vip", "dub", "outtake", "stripped", "rough", "mixdown", "reference", "unmastered", "take",
];

/// Words that start a featured-artist or producer credit.
const CREDIT_WORDS: &[&str] = &["feat", "ft", "featuring", "prod", "produced", "with"];

/// Case, accents and compatibility forms folded: "Beyoncé" and "BEYONCE"
/// fold the same, as do full-width letters and ligatures. Marks are only
/// stripped from Latin, Greek and Cyrillic letters; scripts where a mark
/// changes the letter (Japanese voicing marks, Indic vowel signs) keep it.
pub fn fold_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut base = '\0';
    for c in s.nfkd() {
        if is_combining_mark(c) {
            if base < '\u{0530}' {
                continue;
            }
        } else {
            base = c;
        }
        out.push(c);
    }
    out.nfc().flat_map(char::to_lowercase).collect()
}

/// Folded text as words: "&" reads "and", apostrophes join ("don't" is
/// "dont"), other punctuation splits, and a run of single letters joins
/// ("R.E.M." and "REM" are both "rem").
fn words(folded: &str) -> Vec<String> {
    let mut raw: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in folded.chars() {
        if c.is_alphanumeric() {
            cur.push(c);
        } else if matches!(c, '\'' | '\u{2019}' | '\u{02bc}' | '`') {
            // joins
        } else {
            if !cur.is_empty() {
                raw.push(std::mem::take(&mut cur));
            }
            if c == '&' {
                raw.push("and".into());
            }
        }
    }
    if !cur.is_empty() {
        raw.push(cur);
    }
    let mut out: Vec<String> = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let single = |w: &String| w.chars().count() == 1 && w.chars().all(char::is_alphabetic);
        if single(&raw[i]) {
            let mut j = i;
            while j < raw.len() && single(&raw[j]) {
                j += 1;
            }
            if j - i >= 2 {
                out.push(raw[i..j].concat());
                i = j;
                continue;
            }
        }
        out.push(raw[i].clone());
        i += 1;
    }
    // "Pt. 2", "Pt 2" and "Part 2" are one spelling.
    for k in 0..out.len() {
        if out[k] == "pt" && out.get(k + 1).is_some_and(|n| is_part_number(n)) {
            out[k] = "part".into();
        }
        if ROMAN_PART_EQUALS_NUMBER
            && out[k] == "part"
            && let Some(n) = out.get(k + 1).and_then(|n| roman(n))
        {
            out[k + 1] = n.to_string();
        }
    }
    out
}

fn is_part_number(w: &str) -> bool {
    w.chars().all(|c| c.is_ascii_digit()) || roman(w).is_some()
}

/// A small roman numeral (i to xx).
fn roman(w: &str) -> Option<u32> {
    const R: [&str; 20] = [
        "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv", "xv", "xvi", "xvii",
        "xviii", "xix", "xx",
    ];
    R.iter().position(|r| *r == w).map(|i| i as u32 + 1)
}

fn join(w: &[String]) -> String {
    w.join(" ")
}

/// A version word: one of [`VERSION_WORDS`], or "v2" style.
fn is_version_word(w: &str) -> bool {
    VERSION_WORDS.contains(&w) || is_v_number(w)
}

fn is_v_number(w: &str) -> bool {
    w.len() >= 2 && w.starts_with('v') && w[1..].chars().all(|c| c.is_ascii_digit())
}

/// A credit part. "with" only counts inside brackets ("(with X)"): after a
/// dash it's as likely the title going on ("Song - With You").
fn is_credit(w: &[String], bracketed: bool) -> bool {
    w.first().is_some_and(|f| CREDIT_WORDS.contains(&f.as_str()) && (bracketed || f != "with"))
}

/// Where a featured-artist credit starts in folded text written without
/// brackets ("Song feat. X", "A ft. B"), as a byte offset.
fn credit_start(folded: &str) -> Option<usize> {
    const MARKS: &[&str] = &[" feat. ", " feat ", " ft. ", " ft ", " featuring ", " prod. ", " prod by "];
    MARKS.iter().filter_map(|m| folded.find(m)).filter(|&i| i > 0).min()
}

/// Folded artist: "The Beatles" and "Beatles" match, "A & B" and "A and B"
/// match, "A feat. B" is "a". An empty string for names that don't say who
/// made the song ("Various Artists").
pub fn fold_artist(s: &str) -> String {
    let mut f = fold_text(s);
    if let Some(i) = credit_start(&f) {
        f.truncate(i);
    }
    // "A (feat. B)"
    if let Some(i) = f.find(['(', '['])
        && is_credit(&words(&f[i + 1..]), true)
    {
        f.truncate(i);
    }
    let mut w = words(&f);
    if w.len() > 1 && w[0] == "the" {
        w.remove(0);
    }
    let out = join(&w);
    if NO_ARTIST.contains(&out.as_str()) { String::new() } else { out }
}

/// The featured artists a title credits ("Home (feat. John Legend)",
/// "Song ft. A & B"), folded, sorted. Producer credits don't count.
pub fn featured(title: &str) -> Vec<String> {
    let f = fold_text(title);
    let mut parts: Vec<String> = Vec::new();
    let mut rest = f.as_str();
    while let Some(open) = rest.find(['(', '[', '{']) {
        let close_ch = match rest.as_bytes()[open] {
            b'(' => ')',
            b'[' => ']',
            _ => '}',
        };
        let Some(len) = rest[open + 1..].find(close_ch) else { break };
        let inner = &rest[open + 1..open + 1 + len];
        let w = words(inner);
        if w.first().is_some_and(|x| matches!(x.as_str(), "feat" | "ft" | "featuring" | "with")) {
            parts.push(join(&w[1..]));
        }
        rest = &rest[open + len + 2..];
    }
    for m in [" feat. ", " feat ", " ft. ", " ft ", " featuring "] {
        if let Some(i) = f.find(m) {
            let tail = &f[i + m.len()..];
            let end = tail.find(['(', '[', '{', '-']).unwrap_or(tail.len());
            parts.push(tail[..end].to_string());
        }
    }
    let mut out: Vec<String> = parts
        .iter()
        .flat_map(|p| p.split([',', '&', '/', '+']).flat_map(|x| x.split(" and ")).flat_map(|x| x.split(" x ")).map(str::to_string).collect::<Vec<_>>())
        .map(|n| fold_artist(&n))
        .filter(|n| !n.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// A title split into its parts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Title {
    /// The song's name, folded, without version notes or credits.
    pub title: String,
    /// The version notes, folded, in the order written, joined by "; ".
    /// Empty for the plain song.
    pub version: String,
}

/// Splits a title into the song's name and its version notes. Bracketed
/// parts (round, square or curly) and " - " parts are looked at one by
/// one: a credit ("feat. X", "prod. Y") is dropped, a version note goes to
/// `version`, anything else stays in the name.
pub fn fold_title(s: &str) -> Title {
    let f = fold_text(s);
    let mut name = String::with_capacity(f.len());
    let mut versions: Vec<String> = Vec::new();
    let mut take = |inner: &str, name: &mut String, raw: &str, bracketed: bool| {
        let w = words(inner);
        if w.is_empty() || is_credit(&w, bracketed) {
            return;
        }
        if w.iter().any(|x| is_version_word(x)) {
            versions.push(join(&w));
        } else {
            name.push(' ');
            name.push_str(raw);
        }
    };

    // Brackets, anywhere.
    let mut rest = f.as_str();
    let mut plain = String::with_capacity(f.len());
    while let Some(open) = rest.find(['(', '[', '{']) {
        let close_ch = match rest.as_bytes()[open] {
            b'(' => ')',
            b'[' => ']',
            _ => '}',
        };
        let Some(len) = rest[open + 1..].find(close_ch) else { break };
        plain.push_str(&rest[..open]);
        let inner = &rest[open + 1..open + 1 + len];
        take(inner, &mut plain, inner, true);
        plain.push(' ');
        rest = &rest[open + len + 2..];
    }
    plain.push_str(rest);

    // " - " parts after the first (en and em dashes too).
    let normalized = plain.replace(" \u{2013} ", " - ").replace(" \u{2014} ", " - ");
    let mut parts = normalized.split(" - ");
    name.push_str(parts.next().unwrap_or_default());
    for p in parts {
        take(p, &mut name, p, false);
    }

    // "Song feat. X" without brackets.
    if let Some(i) = credit_start(&name) {
        name.truncate(i);
    }
    let mut title = join(&words(&name));
    if title.is_empty() {
        // A title that is all version note ("(Demo)") keeps it as the name.
        title = join(&words(&f));
        versions.clear();
    }
    Title { title, version: versions.join("; ") }
}

/// The parts of a version note that make another recording, sorted: empty
/// when the notes only say remaster, edition or the like.
pub fn version_core(version_fold: &str) -> Vec<String> {
    let mut core: Vec<String> = version_fold
        .split("; ")
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let w: Vec<&str> = p.split(' ').collect();
            let strong = w.iter().any(|x| STRONG_VERSION_WORDS.contains(x) || is_v_number(x));
            let neutral = w.iter().all(|x| SAME_RECORDING_WORDS.contains(x) || is_number(x));
            strong || !neutral
        })
        .map(str::to_string)
        .collect();
    core.sort();
    core.dedup();
    core
}

/// "2011", or an ordinal like "25th".
fn is_number(w: &str) -> bool {
    let digits = w.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let suffix = &w[digits.len()..];
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && matches!(suffix, "" | "st" | "nd" | "rd" | "th")
}

/// A bare title too common to match across albums ("Intro").
pub fn is_generic(title_fold: &str) -> bool {
    GENERIC.contains(&title_fold)
}

/// Folded album title, version notes ("(Deluxe Edition)") taken off.
pub fn fold_album(s: &str) -> String {
    fold_title(s).title
}
