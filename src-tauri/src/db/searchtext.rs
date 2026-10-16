//! How search reads words: the one place that decides which spellings
//! match each other.
//!
//! Both sides go through it. The typed query is folded here and every word
//! is expanded to all the spellings in its group, so the FTS index (which
//! keeps its own `unicode61` tokens) finds a name written either way. The
//! ranking side folds names with the same rules, so "Thru the Night" and
//! "Through the Nite" are the same title to it.
//!
//! - Case and accents fold away ("Beyoncé" = "beyonce").
//! - Apostrophes and hyphens inside a word join it ("don't" = "dont",
//!   "kay-q" = "kayq"); the parts are kept too, because the index has them
//!   apart ("don" "t").
//! - "$" inside or at the edge of a word reads as "s" ("ri$ky" = "risky").
//! - "&" and "+" read as "and".
//! - Other punctuation separates words.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

/// Spellings that match each other, both ways. The first entry of a group
/// is its canonical form (what ranking compares).
///
/// - Shorthand people type: thru, tho, nite, tonite, lite, u, ur, n ("rock n roll",
///   and "&" / "+"), w (from "w/"), pt, vol, ft / feat, no (from "No.").
/// - Numbers 1 to 10 as digits, words and roman numerals II to X. A lone
///   "I" is left out: it is far more often the word "I" than part one.
///
/// Keep it small: every entry is tried on every search.
pub const VARIANTS: &[&[&str]] = &[
    &["through", "thru"],
    &["though", "tho"],
    &["night", "nite"],
    &["tonight", "tonite"],
    &["light", "lite"],
    &["you", "u"],
    &["your", "ur"],
    &["and", "n"],
    &["with", "w"],
    &["part", "pt"],
    &["volume", "vol"],
    &["featuring", "feat", "ft"],
    &["number", "no"],
    &["1", "one"],
    &["2", "two", "ii"],
    &["3", "three", "iii"],
    &["4", "four", "iv"],
    &["5", "five", "v"],
    &["6", "six", "vi"],
    &["7", "seven", "vii"],
    &["8", "eight", "viii"],
    &["9", "nine", "ix"],
    &["10", "ten", "x"],
];

/// The group a word belongs to, if any.
pub fn group(word: &str) -> Option<&'static [&'static str]> {
    VARIANTS.iter().copied().find(|g| g.contains(&word))
}

/// The canonical spelling of a word (itself when it has no group).
pub fn canon(word: &str) -> &str {
    group(word).map_or(word, |g| g[0])
}

/// Other spellings of a word.
pub fn variants(word: &str) -> impl Iterator<Item = &'static str> + '_ {
    group(word).into_iter().flatten().copied().filter(move |v| *v != word)
}

/// Lower-case, accent-free form.
pub fn normalize(s: &str) -> String {
    if s.is_ascii() {
        return s.to_ascii_lowercase();
    }
    s.nfd().filter(|c| !is_combining_mark(*c)).flat_map(char::to_lowercase).collect()
}

/// One word as search reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    /// Folded and joined: "don't" → "dont", "ri$ky" → "risky".
    pub text: String,
    /// The parts the index stores for it ("don", "t"); empty for "&".
    pub pieces: Vec<String>,
}

/// Apostrophes and hyphens: inside a word they join it.
fn joins(c: char) -> bool {
    matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '\u{02BC}' | '`' | '-' | '\u{2010}' | '\u{2011}')
}

/// Split text into words (see the module notes).
pub fn words(s: &str) -> Vec<Word> {
    let chars: Vec<char> = normalize(s).chars().collect();
    let mut out = Vec::new();
    let mut text = String::new();
    let mut pieces: Vec<String> = Vec::new();
    let mut piece = String::new();
    let end_piece = |piece: &mut String, pieces: &mut Vec<String>| {
        if !piece.is_empty() {
            pieces.push(std::mem::take(piece));
        }
    };
    let flush = |text: &mut String, piece: &mut String, pieces: &mut Vec<String>, out: &mut Vec<Word>| {
        end_piece(piece, pieces);
        if !text.is_empty() {
            out.push(Word { text: std::mem::take(text), pieces: std::mem::take(pieces) });
        }
        pieces.clear();
    };
    for (i, &ch) in chars.iter().enumerate() {
        if ch.is_alphanumeric() {
            text.push(ch);
            piece.push(ch);
            continue;
        }
        let next_alnum = chars.get(i + 1).is_some_and(|c| c.is_alphanumeric());
        if ch == '$' && (next_alnum || !text.is_empty()) {
            text.push('s');
            end_piece(&mut piece, &mut pieces);
            continue;
        }
        if joins(ch) && !text.is_empty() && next_alnum {
            end_piece(&mut piece, &mut pieces);
            continue;
        }
        flush(&mut text, &mut piece, &mut pieces, &mut out);
        if ch == '&' || ch == '+' {
            out.push(Word { text: "and".into(), pieces: Vec::new() });
        }
    }
    flush(&mut text, &mut piece, &mut pieces, &mut out);
    out
}

/// Every form of a name's words that a query word may match: each word
/// (and its canonical spelling), its parts, and each pair of neighbours run together
/// ("Kay Q" also reads as "kayq").
pub fn match_forms(s: &str) -> Vec<String> {
    forms_of(&words(s))
}

/// [`match_forms`] of words already split.
pub fn forms_of(ws: &[Word]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(ws.len() * 2);
    for (i, w) in ws.iter().enumerate() {
        out.push(w.text.clone());
        if canon(&w.text) != w.text {
            out.push(canon(&w.text).to_string());
        }
        if w.pieces.len() > 1 {
            out.extend(w.pieces.iter().cloned());
        }
        if let Some(next) = ws.get(i + 1) {
            out.push(format!("{}{}", w.text, next.text));
        }
    }
    out
}

/// Canonical words of a name joined by spaces (exact and prefix checks).
pub fn canonical(s: &str) -> String {
    canonical_of(&words(s))
}

/// [`canonical`] of words already split.
pub fn canonical_of(ws: &[Word]) -> String {
    ws.iter().map(|w| canon(&w.text)).collect::<Vec<_>>().join(" ")
}

/// Typos a word of `len` chars may have and still match: one wrong,
/// missing, extra or swapped letter from 5 letters, none in shorter words.
pub fn max_typos(len: usize) -> usize {
    usize::from(len >= 5)
}

/// Small words a name often leaves out or words differently. Between
/// other query words they are not required (they still count for ranking).
pub const STOPWORDS: &[&str] = &["the", "a", "an", "of", "to", "in", "on", "at", "by", "for", "my", "is"];

/// Edits between `a` and `b` (insert, delete, substitute, swap two
/// neighbours) when at most `max`, else None.
pub fn edits_within(a: &[char], b: &[char], max: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > max {
        return None;
    }
    let w = b.len() + 1;
    let mut prev2 = vec![0usize; w];
    let mut prev: Vec<usize> = (0..w).collect();
    let mut cur = vec![0usize; w];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut d = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d = d.min(prev2[j - 2] + 1);
            }
            cur[j] = d;
            row_min = row_min.min(d);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(&mut prev2, &mut prev);
        std::mem::swap(&mut prev, &mut cur);
    }
    let d = prev[b.len()];
    (d <= max).then_some(d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(s: &str) -> Vec<String> {
        words(s).into_iter().map(|w| w.text).collect()
    }

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn every_variant_pair_matches_both_ways() {
        for g in VARIANTS {
            for a in *g {
                for b in *g {
                    assert_eq!(canon(a), canon(b), "{a} / {b}");
                    if a != b {
                        assert!(variants(a).any(|v| v == *b), "{a} → {b}");
                    }
                }
            }
        }
        // A word is in one group only.
        let all: Vec<&str> = VARIANTS.iter().flat_map(|g| g.iter().copied()).collect();
        let mut dedup = all.clone();
        dedup.sort_unstable();
        dedup.dedup();
        assert_eq!(all.len(), dedup.len());
        assert_eq!(canonical("Thru the Nite"), canonical("through the night"));
        assert_eq!(canonical("Part II"), canonical("pt 2"));
        assert_eq!(canonical("Vol. 4"), canonical("volume four"));
        assert_eq!(canonical("No. 9"), canonical("number nine"));
        assert_eq!(canonical("Me & U"), canonical("me and you"));
        assert_eq!(canonical("Me + Ur Dog"), canonical("me n your dog"));
        assert_eq!(canonical("Talk w/ Me"), canonical("talk with me"));
        assert_eq!(canonical("Song ft. Somebody"), canonical("song featuring somebody"));
        assert_eq!(canonical("Song feat Somebody"), canonical("song ft somebody"));
        assert_eq!(canonical("Light Tho"), canonical("lite though"));
        assert_eq!(canonical("I"), "i", "a lone I stays a word");
        assert_eq!(canon("unrelated"), "unrelated");
    }

    #[test]
    fn folds_accents_apostrophes_dollar_and_hyphens() {
        assert_eq!(texts("Beyoncé"), vec!["beyonce"]);
        assert_eq!(texts("Don’t Stop"), vec!["dont", "stop"]);
        assert_eq!(words("don't")[0].pieces, vec!["don", "t"]);
        assert_eq!(texts("Ri$ky"), vec!["risky"]);
        assert_eq!(words("Ri$ky")[0].pieces, vec!["ri", "ky"]);
        assert_eq!(texts("$uicide Kid$"), vec!["suicide", "kids"]);
        assert_eq!(texts("Kay-Q"), vec!["kayq"]);
        assert_eq!(texts("rock 'n' roll"), vec!["rock", "n", "roll"]);
        assert_eq!(texts("rockin' all night"), vec!["rockin", "all", "night"]);
        assert_eq!(texts("XY/ZW, Back in Blue!"), vec!["xy", "zw", "back", "in", "blue"]);
        assert_eq!(texts("Up - Down"), vec!["up", "down"]);
        assert_eq!(texts("$ 5"), vec!["5"]);
        assert!(words("&")[0].pieces.is_empty());
        assert!(match_forms("Kay Q").contains(&"kayq".to_string()));
        assert!(match_forms("Hip-Hop").contains(&"hip".to_string()));
    }

    #[test]
    fn edit_distance_with_a_limit() {
        assert_eq!(edits_within(&chars("beatles"), &chars("beatles"), 2), Some(0));
        assert_eq!(edits_within(&chars("beatles"), &chars("beatls"), 1), Some(1));
        assert_eq!(edits_within(&chars("beatles"), &chars("baetles"), 1), Some(1), "swap");
        assert_eq!(edits_within(&chars("beatles"), &chars("beatlesz"), 1), Some(1));
        assert_eq!(edits_within(&chars("beatles"), &chars("btaels"), 1), None);
        assert_eq!(edits_within(&chars("beatles"), &chars("baetls"), 2), Some(2));
        assert_eq!(edits_within(&chars("abc"), &chars("abcdef"), 2), None);
        assert_eq!(max_typos(4), 0);
        assert_eq!(max_typos(5), 1);
        assert_eq!(max_typos(12), 1);
    }
}
