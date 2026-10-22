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
        // A run of single letters reads as one word: "R.E.M." is "rem".
        let single = |w: &Word| w.text.chars().count() == 1 && w.text.chars().all(char::is_alphabetic);
        if single(w) && (i == 0 || !single(&ws[i - 1])) {
            let run: String = ws[i..].iter().take_while(|w| single(w)).map(|w| w.text.as_str()).collect();
            if run.chars().count() > 2 {
                out.push(run);
            }
        }
    }
    out
}

/// Canonical words with each run of single letters run together:
/// "r e m" → "rem", "the b p m band" → "the bpm band".
pub fn join_letters(canonical: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut run = String::new();
    for w in canonical.split(' ') {
        if w.chars().count() == 1 && w.chars().all(char::is_alphabetic) {
            run.push_str(w);
            continue;
        }
        if !run.is_empty() {
            out.push(std::mem::take(&mut run));
        }
        out.push(w.to_string());
    }
    if !run.is_empty() {
        out.push(run);
    }
    out.join(" ")
}

/// The letters of a short word spaced out, as the index stores an
/// abbreviation written with dots ("rem" → "r e m" for "R.E.M."). None for
/// words outside 2 to 4 letters or with anything but letters.
pub fn spelled_out(word: &str) -> Option<String> {
    let n = word.chars().count();
    ((2..=4).contains(&n) && word.chars().all(char::is_alphabetic))
        .then(|| word.chars().map(String::from).collect::<Vec<_>>().join(" "))
}

/// Canonical words of a name joined by spaces (exact and prefix checks).
pub fn canonical(s: &str) -> String {
    canonical_of(&words(s))
}

/// [`canonical`] of words already split.
pub fn canonical_of(ws: &[Word]) -> String {
    ws.iter().map(|w| canon(&w.text)).collect::<Vec<_>>().join(" ")
}

/// A word's letters without its vowels: "world" and "wrld" are both "wrld".
fn skeleton(w: &str) -> String {
    w.chars().filter(|c| !matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y')).collect()
}

/// Two words of a name that are the same as people type them: equal, one
/// written without its vowels ("world" for "WRLD", "mgmt"), or one typo in a
/// word of 5+ letters.
fn word_like(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (sa, sb) = (skeleton(a), skeleton(b));
    if sa == sb && sa.chars().count() >= 2 && (sa == a || sb == b) {
        return true;
    }
    let (ca, cb): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let max = max_typos(ca.len().min(cb.len()));
    max > 0 && edits_within(&ca, &cb, max).is_some()
}

/// True when `name` is the name `query` spells, allowing for how names are
/// written: word for word with stylised spellings and typos ("juice world"
/// for "Juice WRLD"), a leading "the" either way, or the words run together
/// ("kanyewest").
pub fn sounds_like(query: &str, name: &str) -> bool {
    let strip = |s: String| s.strip_prefix("the ").map(str::to_string).unwrap_or(s);
    let (q, n) = (strip(canonical(query)), strip(canonical(name)));
    if q.is_empty() || n.is_empty() {
        return false;
    }
    if q.replace(' ', "") == n.replace(' ', "") {
        return true;
    }
    let (qw, nw): (Vec<&str>, Vec<&str>) = (q.split(' ').collect(), n.split(' ').collect());
    qw.len() == nw.len() && qw.iter().zip(&nw).all(|(a, b)| word_like(a, b))
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
        assert!(match_forms("R.E.M.").contains(&"rem".to_string()));
        assert!(match_forms("The B.P.M. Band").contains(&"bpm".to_string()));
        assert_eq!(join_letters("r e m"), "rem");
        assert_eq!(join_letters("the b p m band x"), "the bpm band x");
        assert_eq!(join_letters("love"), "love");
        assert_eq!(spelled_out("rem").as_deref(), Some("r e m"));
        assert_eq!(spelled_out("a"), None);
        assert_eq!(spelled_out("lovely"), None);
        assert_eq!(spelled_out("b2b"), None);
    }

    #[test]
    fn names_fold_the_same_both_ways() {
        let same = |a: &str, b: &str| assert_eq!(canonical(a), canonical(b), "{a} / {b}");
        same("Tyler, The Creator", "tyler the creator");
        same("A$AP Rocky", "asap rocky");
        same("A$AP Rocky", "a$ap rocky");
        same("Ke$ha", "kesha");
        same("Simon & Garfunkel", "simon and garfunkel");
        same("Simon + Garfunkel", "Simon and Garfunkel");
        same("Beyoncé", "beyonce");
        same("Mr. Oizo", "mr oizo");
        same("Don't Stop", "dont stop");
        same("Thru the Nite", "through the night");
        same("Tho", "though");
        same("U & Ur Friends", "you and your friends");
        same("Pt. 2", "part two");
        same("Part Two", "pt 2");
        same("Seven Nation Army", "7 nation army");
        same("Ten Years", "10 years");
        assert_eq!(canonical("Tyler, The Creator"), "tyler the creator");
        assert_eq!(canonical("A$AP"), "asap");
        // A lone "$" is not a word.
        assert_eq!(canonical("$"), "");
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
