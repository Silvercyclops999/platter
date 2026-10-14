//! Splitting a track's artist line into the artists it credits.
//!
//! Twin of `src/lib/credits.ts` (same rules, same tests): known library
//! artist names are matched first, leftmost and longest, so a name that
//! contains "&" or "," stays whole. What is left is split on ", ", " & ",
//! " feat. ", " ft. ", " featuring ", " x " (any case) and on brackets, so
//! "A (feat. B)" works too.

/// Words that separate credits when they stand alone between names.
const SEP_WORDS: &[&str] = &["featuring", "feat.", "feat", "ft.", "ft", "x"];

fn is_ws(b: u8) -> bool {
    b.is_ascii_whitespace()
}

/// Byte ranges of the separators in `line`. Every separator is ASCII, so
/// byte offsets are always char boundaries.
fn separator_spans(line: &str) -> Vec<(usize, usize)> {
    let s = line.as_bytes();
    let n = s.len();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < n {
        let c = s[i];
        let prev = (i > 0).then(|| s[i - 1]);
        let next = s.get(i + 1).copied();
        if matches!(c, b',' | b'(' | b')' | b'[' | b']') {
            spans.push((i, i + 1));
            i += 1;
            continue;
        }
        if c == b'&' && prev.is_some_and(is_ws) && next.is_some_and(is_ws) {
            spans.push((i, i + 1));
            i += 1;
            continue;
        }
        if prev.is_some_and(|p| is_ws(p) || p == b'(' || p == b'[') {
            let word = SEP_WORDS.iter().find(|w| {
                let end = i + w.len();
                end < n && s[i..end].eq_ignore_ascii_case(w.as_bytes()) && is_ws(s[end])
            });
            if let Some(w) = word {
                // A lone "x" only counts between spaces, never after a bracket.
                if *w != "x" || prev.is_some_and(is_ws) {
                    spans.push((i, i + w.len()));
                    i += w.len();
                    continue;
                }
            }
        }
        i += 1;
    }
    spans
}

/// Whether a name contains something a split would cut at.
pub fn has_separator(name: &str) -> bool {
    !separator_spans(name).is_empty()
}

/// Non-empty name pieces between separators, as trimmed byte ranges.
fn parts(line: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut push = |a: usize, b: usize| {
        let piece = &line[a..b];
        let lead = piece.len() - piece.trim_start().len();
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            out.push((a + lead, a + lead + trimmed.len()));
        }
    };
    for (a, b) in separator_spans(line) {
        push(start, a);
        start = b;
    }
    push(start, line.len());
    out
}

/// Credited names in order, without repeats (case-insensitive). `known`
/// says whether a lower-cased name is a library artist.
pub fn split_credits<'a>(line: &'a str, known: impl Fn(&str) -> bool) -> Vec<&'a str> {
    let ps = parts(line);
    let mut out: Vec<&'a str> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut i = 0;
    while i < ps.len() {
        // Longest run of pieces starting here that is a known name.
        let mut take = (i, line[ps[i].0..ps[i].1].trim());
        for j in (i + 1..ps.len()).rev() {
            let cand = line[ps[i].0..ps[j].1].trim();
            if known(&cand.to_lowercase()) {
                take = (j, cand);
                break;
            }
        }
        let key = take.1.to_lowercase();
        if !seen.contains(&key) {
            seen.push(key);
            out.push(take.1);
        }
        i = take.0 + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(line: &str, known: &[&str]) -> Vec<String> {
        let known: Vec<String> = known.iter().map(|k| k.to_lowercase()).collect();
        split_credits(line, |k| known.iter().any(|n| n == k)).into_iter().map(String::from).collect()
    }

    #[test]
    fn comma_list() {
        assert_eq!(split("Alpha, Beta, Gamma", &[]), ["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn known_names_with_ampersand_or_comma_stay_whole() {
        assert_eq!(split("Moth & Meridian, Saltglass", &["Moth & Meridian"]), ["Moth & Meridian", "Saltglass"]);
        assert_eq!(split("Tyler, The Maker & Rue", &["tyler, the maker"]), ["Tyler, The Maker", "Rue"]);
        assert_eq!(split("Moth & Meridian", &[]), ["Moth", "Meridian"]);
    }

    #[test]
    fn feat_variants() {
        for line in [
            "Alpha feat. Beta",
            "Alpha ft. Beta",
            "Alpha Feat. Beta",
            "Alpha featuring Beta",
            "Alpha (feat. Beta)",
            "Alpha [ft. Beta]",
            "Alpha x Beta",
            "Alpha X Beta",
            "Alpha & Beta",
        ] {
            assert_eq!(split(line, &[]), ["Alpha", "Beta"], "{line}");
        }
        assert_eq!(split("Alpha feat. Beta & Gamma", &[]), ["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn words_inside_names_are_not_separators() {
        assert_eq!(split("Maxx Feather", &[]), ["Maxx Feather"]);
        assert_eq!(split("Rock&Roll Band", &[]), ["Rock&Roll Band"]);
    }

    #[test]
    fn duplicates_collapse() {
        assert_eq!(split("Alpha, alpha, Beta, Alpha", &["alpha"]), ["Alpha", "Beta"]);
    }

    #[test]
    fn empty_line() {
        assert!(split("", &[]).is_empty());
        assert!(split(" , ", &[]).is_empty());
    }
}
