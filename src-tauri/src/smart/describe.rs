//! Describe a playlist: a typed description becomes the editor's rules, on
//! this computer. No service is involved and nothing leaves the app.
//!
//! The words are matched against the library's own names first (genres,
//! styles, moods, artists, albums, eras and version tags from the cache, folded
//! so case, accents and hyphens don't matter), then against a fixed set of
//! phrases for dates ("last summer", "2016", "the 90s"), play stats ("haven't
//! played lately", "never skipped"), length, quality and release status.
//! Words that match nothing are listed as not understood, never guessed. When
//! a word could mean two things the first reading is used and the others are
//! offered as choices.
//!
//! Dates are calendar days in the user's time zone: "last summer" is
//! `Played between 2026-06-01..2026-08-31`, and the evaluator reads the days
//! in local time.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::eval::Row;
use super::rules::{Day, Field, Limit, LimitUnit, MAX_RULES, Op, Rule, RuleGroup, SmartRules, SortBy, range_value};
use crate::identity::fold::fold_text;

/// Longest description read.
pub const MAX_TEXT: usize = 500;
/// Longest name (in words) matched from the library.
const MAX_WORDS: usize = 8;
/// Most other readings offered for one word.
const MAX_OTHERS: usize = 4;

/// Another reading of part of the description.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    /// The words, as typed.
    pub text: String,
    /// The rule they made (index into `rules.rules`, or into the group's rules).
    pub index: usize,
    /// The group the rule is in (index into `rules.groups`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<usize>,
    /// Rules they could make instead.
    pub others: Vec<Rule>,
}

/// `smart_describe`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Described {
    pub title: String,
    pub rules: SmartRules,
    /// Words that matched nothing, as typed (runs of them joined).
    pub unknown: Vec<String>,
    pub choices: Vec<Choice>,
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------- words

#[derive(Debug, Clone)]
struct Tok {
    w: String,
    raw: String,
}

fn fold_word(s: &str) -> String {
    fold_text(s).chars().filter(|c| c.is_alphanumeric() || *c == '+').collect()
}

/// Words, folded: case and accents go, apostrophes join ("haven't" is
/// "havent"), other punctuation splits ("late-night" is two words), "&"
/// reads "and" and "+" is a word of its own.
fn tokens(text: &str) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::new();
    let mut cur = String::new();
    fn flush(cur: &mut String, out: &mut Vec<Tok>) {
        if !cur.is_empty() {
            let w = fold_word(cur);
            if !w.is_empty() {
                out.push(Tok { w, raw: cur.clone() });
            }
            cur.clear();
        }
    }
    for c in text.chars() {
        if c.is_alphanumeric() {
            cur.push(c);
        } else if matches!(c, '\'' | '\u{2019}' | '\u{2018}' | '`' | '\u{02bc}') {
            // Joins; a leading one ('90s) is dropped.
        } else {
            flush(&mut cur, &mut out);
            match c {
                '&' => out.push(Tok { w: "and".into(), raw: "&".into() }),
                '+' => out.push(Tok { w: "+".into(), raw: "+".into() }),
                _ => {}
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

fn words_of(name: &str) -> Vec<String> {
    tokens(name).into_iter().map(|t| t.w).collect()
}

// ---------------------------------------------------------------- the library's names

#[derive(Debug, Clone)]
struct Entry {
    words: Vec<String>,
    field: Field,
    name: String,
}

/// The library's own names, folded for matching.
#[derive(Debug, Default)]
pub struct Vocabulary {
    exact: HashMap<String, Vec<(Field, String)>>,
    /// Artists and eras by their first word, for "Kanye" or "Yandhi".
    starts: HashMap<String, Vec<Entry>>,
    seen: HashSet<(Field, String)>,
}

/// Which reading of a name comes first.
fn rank(f: Field) -> u8 {
    match f {
        Field::Genre => 0,
        Field::Mood => 1,
        Field::Style => 2,
        Field::Artist => 3,
        Field::Album => 4,
        Field::Era => 5,
        _ => 6,
    }
}

impl Vocabulary {
    pub fn add(&mut self, field: Field, name: &str) {
        let name = name.trim();
        let words = words_of(name);
        if words.is_empty() || words.len() > MAX_WORDS || !self.seen.insert((field, words.join(" "))) {
            return;
        }
        let mut keys = vec![words.join(" ")];
        if words.len() > 1 {
            // "Hip-Hop" also reads "hiphop".
            keys.push(words.concat());
        }
        for k in keys {
            let list = self.exact.entry(k).or_default();
            if !list.iter().any(|(f, _)| *f == field) {
                list.push((field, name.to_string()));
                list.sort_by_key(|(f, _)| rank(*f));
            }
        }
        if field == Field::Artist && words.len() > 1 && words[0] == "the" {
            // "beatles" for "The Beatles".
            let key = words[1..].join(" ");
            let list = self.exact.entry(key).or_default();
            if !list.iter().any(|(f, _)| *f == field) {
                list.push((field, name.to_string()));
                list.sort_by_key(|(f, _)| rank(*f));
            }
        }
        if matches!(field, Field::Artist | Field::Era) && words.len() > 1 {
            self.starts.entry(words[0].clone()).or_default().push(Entry { words, field, name: name.to_string() });
        }
    }

    /// Every name in the library cache.
    pub fn from_rows(rows: &[Row]) -> Self {
        let mut v = Self::default();
        for r in rows {
            for g in &r.genres {
                v.add(Field::Genre, g);
            }
            for s in &r.styles {
                v.add(Field::Style, s);
            }
            for m in &r.moods {
                v.add(Field::Mood, m);
            }
            v.add(Field::Artist, &r.artist);
            if let Some(a) = &r.track_artist {
                v.add(Field::Artist, a);
            }
            v.add(Field::Album, &r.album);
            if let Some(e) = &r.era {
                v.add(Field::Era, e);
            }
            if let Some(x) = &r.version {
                v.add(Field::Version, x);
            }
        }
        v
    }

    fn lookup(&self, words: &[String]) -> Vec<(Field, String)> {
        self.exact.get(&words.join(" ")).cloned().unwrap_or_default()
    }

    /// A name of this field, by its folded spelling.
    fn named(&self, field: Field, name: &str) -> Option<String> {
        let w = words_of(name);
        self.exact.get(&w.join(" "))?.iter().find(|(f, _)| *f == field).map(|(_, n)| n.clone())
    }

    /// Artists or eras whose name starts with these words ("kanye" for "Kanye West").
    fn starting(&self, words: &[String], field: Field) -> Vec<String> {
        let Some(first) = words.first() else { return Vec::new() };
        let mut out: Vec<String> = self
            .starts
            .get(first)
            .into_iter()
            .flatten()
            .filter(|e| e.field == field && e.words.len() > words.len() && e.words.starts_with(words))
            .map(|e| e.name.clone())
            .collect();
        out.sort();
        out.dedup();
        out
    }
}

// ---------------------------------------------------------------- word lists

const FILLER: &[&str] = &[
    "a", "an", "the", "some", "my", "me", "i", "im", "ive", "id", "for", "of", "with", "from", "in", "on", "at", "to", "that", "which", "who", "are", "is",
    "was", "were", "be", "been", "and", "but", "so", "also", "plus", "songs", "song", "tracks", "track", "music", "stuff", "tunes", "jams", "things", "playlist",
    "mix", "only", "just", "all", "any", "by", "please", "give", "make", "play", "want", "really", "very", "kind", "sort", "bit", "lots", "lot", "vibes", "vibe",
    "style", "type", "esque", "ish", "like", "sounding", "sounds", "artists", "it", "them", "these", "those", "this", "era", "eras", "stuff", "etc", "something",
    "anything", "everything", "mostly", "mainly", "good", "great", "nice", "favourite", "can", "you", "get", "pick", "put", "together", "one", "ones", "where",
    "when", "while", "has", "have", "had", "do", "does", "would", "should", "let", "lets", "about",
];

/// Release type words.
const RELEASE: &[(&str, &str)] = &[
    ("album", "albums"),
    ("albums", "albums"),
    ("lp", "albums"),
    ("lps", "albums"),
    ("single", "singlesEps"),
    ("singles", "singlesEps"),
    ("ep", "singlesEps"),
    ("eps", "singlesEps"),
    ("live", "live"),
    ("compilation", "compilations"),
    ("compilations", "compilations"),
    ("soundtrack", "soundtracks"),
    ("soundtracks", "soundtracks"),
    ("ost", "soundtracks"),
    ("osts", "soundtracks"),
    ("score", "soundtracks"),
    ("scores", "soundtracks"),
    ("remix", "remixes"),
    ("remixes", "remixes"),
    ("remixed", "remixes"),
    ("demo", "demos"),
    ("demos", "demos"),
    ("mixtape", "mixtapes"),
    ("mixtapes", "mixtapes"),
];

/// Mood and genre words read through the library's own names, first one
/// the library has wins.
const LIKE: &[(&str, &[&str])] = &[
    ("chill", &["Chill", "Mellow", "Relaxed", "Calm", "Laid-Back", "Chillout"]),
    ("chilled", &["Chill", "Mellow", "Relaxed", "Calm", "Laid-Back"]),
    ("relaxing", &["Relaxed", "Calm", "Mellow", "Chill", "Soothing"]),
    ("mellow", &["Mellow", "Chill", "Relaxed", "Calm"]),
    ("calm", &["Calm", "Mellow", "Relaxed", "Peaceful"]),
    ("upbeat", &["Upbeat", "Energetic", "Happy", "Cheerful", "Lively"]),
    ("energetic", &["Energetic", "Upbeat", "Intense", "Lively"]),
    ("hype", &["Energetic", "Intense", "Aggressive", "Rowdy"]),
    ("happy", &["Happy", "Cheerful", "Joyous", "Upbeat"]),
    ("sad", &["Sad", "Melancholy", "Gloomy", "Bittersweet", "Somber"]),
    ("melancholy", &["Melancholy", "Sad", "Bittersweet"]),
    ("dark", &["Dark", "Brooding", "Ominous", "Gloomy"]),
    ("angry", &["Angry", "Aggressive", "Hostile", "Intense"]),
    ("aggressive", &["Aggressive", "Angry", "Intense"]),
    ("romantic", &["Romantic", "Sensual", "Passionate", "Tender"]),
    ("dreamy", &["Dreamy", "Ethereal", "Atmospheric"]),
    ("party", &["Party", "Celebratory", "Rowdy", "Fun"]),
    ("workout", &["Energetic", "Intense", "Workout"]),
    ("gym", &["Energetic", "Intense", "Workout"]),
    ("focus", &["Calm", "Reflective", "Meditative", "Focus"]),
    ("study", &["Calm", "Reflective", "Meditative", "Study"]),
    ("sleep", &["Calm", "Peaceful", "Soothing", "Sleep"]),
    ("night", &["Night", "Late Night", "Nocturnal"]),
    ("nighttime", &["Night", "Late Night", "Nocturnal"]),
    ("hiphop", &["Hip-Hop", "Hip Hop", "Rap"]),
    ("rnb", &["R&B", "Rhythm & Blues"]),
    ("lofi", &["Lo-Fi", "Lofi", "Lo-Fi Hip Hop"]),
    ("edm", &["Electronic", "Dance", "EDM"]),
];

const LIKE2: &[(&str, &str, &[&str])] = &[
    ("late", "night", &["Late Night", "Nocturnal", "Night", "Midnight"]),
    ("love", "songs", &["Romantic", "Love"]),
    ("feel", "good", &["Feel Good", "Happy", "Upbeat"]),
    ("hip", "hop", &["Hip-Hop", "Hip Hop", "Rap"]),
    ("r", "b", &["R&B", "Rhythm & Blues"]),
    ("lo", "fi", &["Lo-Fi", "Lofi"]),
];

/// Version words read even when no file in the library has them yet.
const VERSIONS: &[(&str, &str)] = &[
    ("remaster", "remaster"),
    ("remasters", "remaster"),
    ("remastered", "remaster"),
    ("acoustic", "acoustic"),
    ("instrumental", "instrumental"),
    ("instrumentals", "instrumental"),
    ("unplugged", "unplugged"),
    ("acapella", "a cappella"),
    ("acappella", "a cappella"),
    ("stripped", "stripped"),
    ("extended", "extended"),
];

const MONTHS: &[&str] = &["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];
const MONTHS_SHORT: &[&str] = &["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn month_of(w: &str) -> Option<u32> {
    MONTHS.iter().position(|m| *m == w).or_else(|| MONTHS_SHORT.iter().position(|m| *m == w && w != "mar" && w != "may")).map(|i| i as u32 + 1)
}

fn number(w: &str) -> Option<u32> {
    if let Ok(n) = w.parse::<u32>() {
        return Some(n);
    }
    let words = [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen",
        "seventeen", "eighteen", "nineteen", "twenty",
    ];
    if let Some(i) = words.iter().position(|x| *x == w) {
        return Some(i as u32);
    }
    match w {
        "thirty" => Some(30),
        "forty" => Some(40),
        "fifty" => Some(50),
        "hundred" => Some(100),
        "once" => Some(1),
        "twice" => Some(2),
        _ => None,
    }
}

fn year_of(w: &str) -> Option<i32> {
    (w.len() == 4).then(|| w.parse::<i32>().ok()).flatten().filter(|y| (1900..=2099).contains(y))
}

/// "90s", "1990s", "nineties": the decade's first year, and whether a
/// two-digit one could also be a century earlier ("20s").
fn decade_of(w: &str) -> Option<(i32, bool)> {
    let named = [
        ("fifties", 1950),
        ("sixties", 1960),
        ("seventies", 1970),
        ("eighties", 1980),
        ("nineties", 1990),
        ("noughties", 2000),
        ("aughts", 2000),
        ("twenties", 2020),
        ("thirties", 1930),
        ("forties", 1940),
    ];
    if let Some((_, y)) = named.iter().find(|(n, _)| *n == w) {
        return Some((*y, *y == 2020));
    }
    let d = w.strip_suffix('s')?;
    if !d.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: i32 = d.parse().ok()?;
    match d.len() {
        4 if n % 10 == 0 && (1900..=2090).contains(&n) => Some((n, false)),
        2 if n % 10 == 0 => {
            let y = if n <= 20 { 2000 + n } else { 1900 + n };
            Some((y, (10..=20).contains(&n)))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------- calendar

fn days(d: Day) -> i64 {
    crate::stats::days_from_civil(d.0 as i64, d.1, d.2)
}

fn day(n: i64) -> Day {
    let (y, m, d) = crate::stats::civil_from_days(n);
    (y as i32, m, d)
}

fn month_end(y: i32, m: u32) -> Day {
    let next = if m == 12 { (y + 1, 1, 1) } else { (y, m + 1, 1) };
    day(days(next) - 1)
}

/// A season's days in the year it starts (winter runs into the next year).
fn season(name: &str, y: i32) -> Option<(Day, Day)> {
    Some(match name {
        "spring" => ((y, 3, 1), (y, 5, 31)),
        "summer" => ((y, 6, 1), (y, 8, 31)),
        "fall" | "autumn" => ((y, 9, 1), (y, 11, 30)),
        "winter" => ((y, 12, 1), month_end(y + 1, 2)),
        _ => return None,
    })
}

/// The latest season that has ended before today.
fn last_season(name: &str, today: Day) -> Option<(Day, Day)> {
    (0..3).map(|k| today.0 - k).filter_map(|y| season(name, y)).find(|(_, end)| *end < today)
}

/// The season around today, or this year's.
fn this_season(name: &str, today: Day) -> Option<(Day, Day)> {
    if name == "winter" {
        let y = if today.1 == 12 { today.0 } else { today.0 - 1 };
        return season(name, y);
    }
    season(name, today.0)
}

// ---------------------------------------------------------------- the parser

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ctx {
    /// Years mean the release year.
    Release,
    /// Times mean plays.
    Played,
    /// Times mean when it was added.
    Added,
}

struct Parser<'a> {
    toks: Vec<Tok>,
    i: usize,
    vocab: &'a Vocabulary,
    today: Day,
    rules: Vec<Rule>,
    choices: Vec<Choice>,
    unknown: Vec<usize>,
    notes: Vec<String>,
    negate: bool,
    ctx: Ctx,
    or_seen: bool,
    sort: SortBy,
    limit: Option<Limit>,
}

fn rule(field: Field, op: Op, value: impl Into<String>) -> Rule {
    Rule { field, op, value: value.into(), plex_field: None }
}

fn flip(mut r: Rule) -> Rule {
    use super::rules::Kind;
    if r.field.kind() == Kind::Bool {
        r.value = if r.value == "true" { "false".into() } else { "true".into() };
        return r;
    }
    r.op = match r.op {
        Op::Is => Op::IsNot,
        Op::IsNot => Op::Is,
        Op::Contains => Op::NotContains,
        Op::NotContains => Op::Contains,
        Op::InLast => Op::NotInLast,
        Op::NotInLast => Op::InLast,
        Op::Gt => Op::Lt,
        Op::Lt => Op::Gt,
        // Describing never makes these.
        Op::Between | Op::BeginsWith | Op::EndsWith => r.op,
    };
    r
}

fn tag_rule(field: Field, name: &str) -> Rule {
    let op = if field == Field::Version { Op::Contains } else { Op::Is };
    rule(field, op, name)
}

impl<'a> Parser<'a> {
    fn w(&self, k: usize) -> &str {
        self.toks.get(self.i + k).map(|t| t.w.as_str()).unwrap_or("")
    }

    /// The words from here match `pat`, each item a `|`-separated set.
    fn seq(&self, at: usize, pat: &[&str]) -> bool {
        pat.iter().enumerate().all(|(k, p)| {
            let w = self.toks.get(self.i + at + k).map(|t| t.w.as_str()).unwrap_or("");
            !w.is_empty() && p.split('|').any(|x| x == w)
        })
    }

    fn text(&self, from: usize, n: usize) -> String {
        self.toks[from..from + n].iter().map(|t| t.raw.as_str()).collect::<Vec<_>>().join(" ")
    }

    fn push(&mut self, r: Rule) -> Option<usize> {
        let r = if std::mem::take(&mut self.negate) { flip(r) } else { r };
        self.ctx = Ctx::Release;
        if let Some(i) = self.rules.iter().position(|x| *x == r) {
            return Some(i);
        }
        if self.rules.len() >= MAX_RULES {
            return None;
        }
        self.rules.push(r);
        Some(self.rules.len() - 1)
    }

    /// Push a rule read from `n` words, with other readings.
    fn push_with(&mut self, r: Rule, others: Vec<Rule>, n: usize) {
        let negate = self.negate;
        if let Some(index) = self.push(r)
            && !others.is_empty()
        {
            let others = others.into_iter().take(MAX_OTHERS).map(|o| if negate { flip(o) } else { o }).collect();
            self.choices.push(Choice { text: self.text(self.i, n), index, group: None, others });
        }
    }

    fn range(&mut self, field: Field, (a, b): (Day, Day)) {
        let b = b.min(self.today).max(a);
        self.push(rule(field, Op::Between, range_value(a, b)));
    }

    /// The field a time phrase is about.
    fn time_field(&self) -> Field {
        if self.ctx == Ctx::Added { Field::AddedAt } else { Field::Played }
    }

    fn span_field(&self) -> Field {
        if self.ctx == Ctx::Added { Field::AddedAt } else { Field::LastPlayed }
    }

    fn run(&mut self) {
        while self.i < self.toks.len() {
            let n = self
                .phrase()
                .or_else(|| self.library(2))
                .or_else(|| self.keyword())
                .or_else(|| self.filler())
                .or_else(|| self.library(1))
                .or_else(|| self.alike())
                .or_else(|| self.version_word());
            match n {
                Some(n) => self.i += n.max(1),
                None => {
                    self.unknown.push(self.i);
                    self.negate = false;
                    self.i += 1;
                }
            }
        }
    }

    fn filler(&mut self) -> Option<usize> {
        let w = self.w(0);
        if w == "or" {
            self.or_seen = true;
            return Some(1);
        }
        FILLER.contains(&w).then_some(1)
    }

    /// A library name of at least `min` words, longest first.
    fn library(&mut self, min: usize) -> Option<usize> {
        let left = self.toks.len() - self.i;
        for n in (min..=MAX_WORDS.min(left)).rev() {
            let words: Vec<String> = self.toks[self.i..self.i + n].iter().map(|t| t.w.clone()).collect();
            if n == 1 && FILLER.contains(&words[0].as_str()) {
                return None;
            }
            let mut hits = self.vocab.lookup(&words);
            if hits.is_empty() && n == 1 {
                // "ballads" for "Ballad".
                if let Some(s) = words[0].strip_suffix('s').filter(|s| s.len() >= 3) {
                    hits = self.vocab.lookup(&[s.to_string()]);
                }
            }
            if hits.is_empty() && n == 1 && words[0].len() >= 3 {
                // "Kanye" for "Kanye West".
                let artists = self.vocab.starting(&words, Field::Artist);
                if let Some((first, rest)) = artists.split_first() {
                    let others = rest.iter().map(|a| rule(Field::Artist, Op::Is, a.clone())).collect();
                    self.push_with(rule(Field::Artist, Op::Is, first.clone()), others, n);
                    return Some(n);
                }
            }
            if let Some(((f, name), rest)) = hits.split_first() {
                let others = rest.iter().map(|(f, n)| tag_rule(*f, n)).collect();
                self.push_with(tag_rule(*f, name), others, n);
                return Some(n);
            }
        }
        None
    }

    /// Mood and genre words through the library's names.
    fn alike(&mut self) -> Option<usize> {
        let (n, names): (usize, &[&str]) = if let Some((_, _, names)) = LIKE2.iter().find(|(a, b, _)| self.seq(0, &[*a, *b])) {
            (2, names)
        } else {
            (1, LIKE.iter().find(|(w, _)| *w == self.w(0))?.1)
        };
        let mut found: Vec<Rule> = Vec::new();
        for name in names {
            for f in [Field::Mood, Field::Genre, Field::Style] {
                if let Some(real) = self.vocab.named(f, name) {
                    let r = tag_rule(f, &real);
                    if !found.contains(&r) {
                        found.push(r);
                    }
                }
            }
        }
        // Nothing in the library by that name: not understood.
        let (first, rest) = found.split_first()?;
        self.push_with(first.clone(), rest.to_vec(), n);
        Some(n)
    }

    fn version_word(&mut self) -> Option<usize> {
        if self.seq(0, &["radio", "edit|edits"]) {
            self.push(rule(Field::Version, Op::Contains, "radio edit"));
            return Some(2);
        }
        let (_, v) = VERSIONS.iter().find(|(w, _)| *w == self.w(0))?;
        self.push(rule(Field::Version, Op::Contains, *v));
        Some(1)
    }

    /// Single words with a fixed meaning.
    fn keyword(&mut self) -> Option<usize> {
        let w = self.w(0).to_string();
        let w = w.as_str();
        match w {
            "no" | "not" | "without" | "except" | "excluding" | "minus" | "nothing" | "non" | "never" | "exclude" => {
                self.negate = !self.negate;
                return Some(1);
            }
            "unreleased" | "leak" | "leaks" | "leaked" | "vault" | "outtake" | "outtakes" | "unofficial" | "scrapped" | "shelved" | "snippet" | "snippets" => {
                self.push(rule(Field::Unreleased, Op::Is, "true"));
                return Some(1);
            }
            "released" | "official" | "retail" => {
                self.push(rule(Field::Unreleased, Op::Is, "false"));
                return Some(1);
            }
            "lossless" | "flac" | "alac" | "hires" | "hifi" => {
                self.push(rule(Field::Lossless, Op::Is, "true"));
                return Some(1);
            }
            "lossy" | "mp3" | "mp3s" | "aac" | "compressed" => {
                self.push(rule(Field::Lossless, Op::Is, "false"));
                return Some(1);
            }
            "explicit" | "dirty" | "uncensored" | "nsfw" => {
                self.push(rule(Field::Explicit, Op::Is, "true"));
                return Some(1);
            }
            "clean" | "censored" => {
                self.push(rule(Field::Explicit, Op::Is, "false"));
                return Some(1);
            }
            "liked" | "loved" | "favorite" | "favorites" | "favourites" | "faves" | "hearted" => {
                self.push(rule(Field::Liked, Op::Is, "true"));
                self.ctx = Ctx::Played;
                return Some(1);
            }
            "unplayed" => {
                self.push(rule(Field::PlayCount, Op::Is, "0"));
                return Some(1);
            }
            "unrated" => {
                self.push(rule(Field::Rating, Op::Is, "0"));
                return Some(1);
            }
            "unfinished" | "incomplete" => {
                self.push(rule(Field::Finished, Op::Is, "false"));
                return Some(1);
            }
            "finished" | "completed" => {
                self.push(rule(Field::Finished, Op::Is, "true"));
                return Some(1);
            }
            "skipped" | "skips" => {
                self.push(rule(Field::Skips, Op::Gt, "0"));
                return Some(1);
            }
            "played" | "heard" | "listened" | "listen" | "playing" | "listening" | "spun" => {
                self.ctx = Ctx::Played;
                return Some(1);
            }
            "added" => {
                self.ctx = Ctx::Added;
                return Some(1);
            }
            "new" | "fresh" => {
                self.push(rule(Field::AddedAt, Op::InLast, "30d"));
                return Some(1);
            }
            "random" | "randomly" | "shuffled" | "shuffle" => {
                self.sort = SortBy::Random;
                return Some(1);
            }
            "short" | "shorter" | "quick" => {
                self.push(rule(Field::Duration, Op::Lt, "3"));
                return Some(1);
            }
            "long" | "longer" | "epic" | "lengthy" => {
                self.push(rule(Field::Duration, Op::Gt, "6"));
                return Some(1);
            }
            "oldies" => {
                self.push(rule(Field::Year, Op::Lt, "1980"));
                return Some(1);
            }
            "lately" | "recently" | "recent" => {
                let span = if self.ctx == Ctx::Added { "30d" } else { "3m" };
                let f = self.span_field();
                self.push(rule(f, Op::InLast, span));
                return Some(1);
            }
            _ => {}
        }
        if let Some((_, t)) = RELEASE.iter().find(|(k, _)| *k == w) {
            let r = rule(Field::ReleaseType, Op::Is, *t);
            // "demos" may also be a version tag the library has.
            let mut others = Vec::new();
            for v in [w, w.strip_suffix("es").unwrap_or(w), w.strip_suffix('s').unwrap_or(w)] {
                if let Some(name) = self.vocab.named(Field::Version, v) {
                    let o = rule(Field::Version, Op::Contains, name);
                    if !others.contains(&o) {
                        others.push(o);
                    }
                }
            }
            self.push_with(r, others, 1);
            return Some(1);
        }
        if let Some((y, other)) = decade_of(w) {
            let others = if other { vec![rule(Field::Decade, Op::Is, (y - 100).to_string())] } else { vec![] };
            self.push_with(rule(Field::Decade, Op::Is, y.to_string()), others, 1);
            return Some(1);
        }
        None
    }

    /// Fixed phrases, tried first.
    fn phrase(&mut self) -> Option<usize> {
        self.plays()
            .or_else(|| self.skips())
            .or_else(|| self.likes())
            .or_else(|| self.finished())
            .or_else(|| self.era())
            .or_else(|| self.length())
            .or_else(|| self.counts())
            .or_else(|| self.rating())
            .or_else(|| self.quality())
            .or_else(|| self.decade_part())
            .or_else(|| self.years())
            .or_else(|| self.periods())
            .or_else(|| self.sorts())
    }

    /// "lately", "in a while", "in the last 2 weeks", "this year": a span
    /// back from today, and the words it took.
    fn span_at(&self, at: usize) -> Option<(usize, String)> {
        if self.seq(at, &["lately|recently"]) {
            return Some((1, "3m".into()));
        }
        if self.seq(at, &["in|for", "a|ages|forever", "while"]) || self.seq(at, &["in|for", "a", "bit"]) {
            return Some((3, "6m".into()));
        }
        if self.seq(at, &["in|for", "ages|forever|years"]) {
            return Some((2, "1y".into()));
        }
        if self.seq(at, &["in|for", "a", "long", "time"]) {
            return Some((4, "1y".into()));
        }
        if self.seq(at, &["this", "year"]) {
            let since = days(self.today) - days((self.today.0, 1, 1)) + 1;
            return Some((2, format!("{since}d")));
        }
        if self.seq(at, &["this", "month"]) {
            return Some((2, format!("{}d", self.today.2)));
        }
        // "in the last 2 weeks", "in the past year", "over the last month", "the last 30 days"
        let mut k = at;
        if self.seq(k, &["in|over|during|within"]) {
            k += 1;
        }
        if self.seq(k, &["the"]) {
            k += 1;
        }
        if self.seq(k, &["last|past"]) {
            k += 1;
            let (n, used) = match self.toks.get(self.i + k).and_then(|t| number(&t.w)) {
                Some(n) => (n, 1),
                None => (1, 0),
            };
            let unit = self.toks.get(self.i + k + used).map(|t| t.w.as_str()).unwrap_or("");
            let u = match unit {
                "day" | "days" => "d",
                "week" | "weeks" => "w",
                "month" | "months" => "m",
                "year" | "years" => "y",
                _ => return None,
            };
            if n == 0 || (used == 0 && k == at + 1) {
                // A bare "last week" is a calendar week (see periods).
                return None;
            }
            return Some((k + used + 1 - at, format!("{n}{u}")));
        }
        None
    }

    /// "haven't played lately", "never played", "played in the last month".
    fn plays(&mut self) -> Option<usize> {
        if self.seq(0, &["most|top", "played|heard|listened"]) || self.seq(0, &["played", "the", "most"]) || self.seq(0, &["on", "repeat"]) {
            self.sort = SortBy::MostPlayed;
            return Some(if self.w(0) == "played" { 3 } else { 2 });
        }
        if self.seq(0, &["least|rarely|barely|seldom|hardly", "played|heard|listened"]) {
            if self.w(0) == "least" {
                self.sort = SortBy::LeastPlayed;
            } else {
                self.push(rule(Field::PlayCount, Op::Lt, "3"));
            }
            return Some(2);
        }
        if self.seq(0, &["played|heard", "a", "lot"]) || self.seq(0, &["often|heavily", "played"]) {
            self.push(rule(Field::PlayCount, Op::Gt, "9"));
            return Some(if self.w(0) == "played" { 3 } else { 2 });
        }
        // The negative forms.
        let mut k = 0;
        if self.seq(0, &["have|did|do|has", "not"]) {
            k = 2;
        } else if self.seq(0, &["havent|never|didnt|dont|hasnt|not|aint"]) {
            k = 1;
        }
        if k > 0 {
            let mut j = k;
            if self.seq(j, &["been|ever|really|yet|even"]) {
                j += 1;
            }
            if self.seq(j, &["played|heard|listened|play|hear|listen|played"]) {
                j += 1;
                if self.seq(j, &["to"]) {
                    j += 1;
                }
                if self.seq(j, &["it|them|much|yet|before"]) {
                    j += 1;
                }
                if let Some((n, span)) = self.span_at(j) {
                    self.push(rule(Field::LastPlayed, Op::NotInLast, span));
                    return Some(j + n);
                }
                self.push(rule(Field::PlayCount, Op::Is, "0"));
                return Some(j);
            }
            return None;
        }
        // "played in the last month", "played lately"
        if self.seq(0, &["played|heard|listened|playing"]) {
            let mut j = 1;
            if self.seq(j, &["to"]) {
                j += 1;
            }
            if let Some((n, span)) = self.span_at(j) {
                self.push(rule(Field::LastPlayed, Op::InLast, span));
                return Some(j + n);
            }
        }
        if self.ctx == Ctx::Played
            && let Some((n, span)) = self.span_at(0)
        {
            self.push(rule(Field::LastPlayed, Op::InLast, span));
            return Some(n);
        }
        if self.ctx == Ctx::Added
            && let Some((n, span)) = self.span_at(0)
        {
            let span = if span == "3m" { "30d".to_string() } else { span };
            self.push(rule(Field::AddedAt, Op::InLast, span));
            return Some(n);
        }
        // "in the last 30 days", "over the past 2 weeks"
        if self.seq(0, &["in|over|during|within|the|last|past"])
            && let Some((n, span)) = self.span_at(0)
        {
            let f = self.span_field();
            self.push(rule(f, Op::InLast, span));
            return Some(n);
        }
        if self.seq(0, &["recently|newly|just", "added"]) {
            self.push(rule(Field::AddedAt, Op::InLast, "30d"));
            return Some(2);
        }
        if self.seq(0, &["added", "lately|recently"]) {
            self.push(rule(Field::AddedAt, Op::InLast, "30d"));
            return Some(2);
        }
        // "played more than 10 times", "10+ plays", "played twice"
        if self.seq(0, &["played"]) {
            let mut j = 1;
            if let Some((n, op, v)) = self.compare_at(j) {
                j += n;
                if self.seq(j, &["times|time|plays"]) {
                    self.push(rule(Field::PlayCount, op, v.to_string()));
                    return Some(j + 1);
                }
            }
            if let Some(n) = self.toks.get(self.i + 1).filter(|t| t.w == "once" || t.w == "twice").and_then(|t| number(&t.w)) {
                self.push(rule(Field::PlayCount, Op::Is, n.to_string()));
                return Some(2);
            }
        }
        None
    }

    /// "more than 10", "under 3", "at least 5": how many words, the
    /// comparison and the number (already adjusted for "at least").
    fn compare_at(&self, at: usize) -> Option<(usize, Op, f64)> {
        let num_at = |k: usize| self.toks.get(self.i + k).and_then(|t| number(&t.w));
        let (n, op, adj) = if self.seq(at, &["more|longer|greater|higher", "than"]) {
            (2, Op::Gt, 0.0)
        } else if self.seq(at, &["less|fewer|shorter|lower", "than"]) {
            (2, Op::Lt, 0.0)
        } else if self.seq(at, &["at", "least"]) {
            (2, Op::Gt, -1.0)
        } else if self.seq(at, &["at", "most"]) || self.seq(at, &["up", "to"]) {
            (2, Op::Lt, 1.0)
        } else if self.seq(at, &["over|above|past|exceeding"]) {
            (1, Op::Gt, 0.0)
        } else if self.seq(at, &["under|below|within|max|maximum"]) {
            (1, Op::Lt, 0.0)
        } else if self.seq(at, &["min|minimum"]) {
            (1, Op::Gt, -1.0)
        } else {
            return None;
        };
        let v = num_at(at + n)? as f64;
        Some((n + 1, op, v + adj))
    }

    fn skips(&mut self) -> Option<usize> {
        if self.seq(0, &["never|dont|didnt|not|rarely|seldom", "skip|skipped|skips"]) || self.seq(0, &["do|did", "not", "skip|skipped"]) {
            let n = if self.w(1) == "not" { 3 } else { 2 };
            let r = if matches!(self.w(0), "rarely" | "seldom") { rule(Field::Skips, Op::Lt, "2") } else { rule(Field::Skips, Op::Is, "0") };
            self.push(r);
            return Some(n);
        }
        if self.seq(0, &["skip|skipped|skips"]) {
            let mut j = 1;
            if let Some((n, op, v)) = self.compare_at(j) {
                j += n;
                if self.seq(j, &["times|time"]) {
                    j += 1;
                }
                self.push(rule(Field::Skips, op, format!("{v}")));
                return Some(j);
            }
            if self.seq(1, &["a", "lot"]) || self.seq(1, &["often|lots"]) {
                self.push(rule(Field::Skips, Op::Gt, "2"));
                return Some(if self.w(1) == "a" { 3 } else { 2 });
            }
            self.push(rule(Field::Skips, Op::Gt, "0"));
            return Some(1);
        }
        if self.seq(0, &["often|always|usually", "skip|skipped"]) {
            self.push(rule(Field::Skips, Op::Gt, "2"));
            return Some(2);
        }
        None
    }

    fn likes(&mut self) -> Option<usize> {
        if self.seq(0, &["i|ive|we", "dont|didnt|never", "like|love|liked|loved"]) {
            self.push(rule(Field::Liked, Op::Is, "false"));
            return Some(3);
        }
        if self.seq(0, &["dont|didnt", "like|love"]) || self.seq(0, &["dislike|disliked|hate|hated"]) {
            self.push(rule(Field::Liked, Op::Is, "false"));
            return Some(if matches!(self.w(0), "dont" | "didnt") { 2 } else { 1 });
        }
        if self.seq(0, &["i|ive|we", "like|love|liked|loved|adore|adored|saved|liking|loving"]) || self.seq(0, &["i|we", "really|truly", "like|love|liked|loved"]) {
            let n = if matches!(self.w(1), "really" | "truly") { 3 } else { 2 };
            self.push(rule(Field::Liked, Op::Is, "true"));
            self.ctx = Ctx::Played;
            return Some(n);
        }
        None
    }

    fn finished(&mut self) -> Option<usize> {
        if self.seq(0, &["never|not|didnt|havent|hasnt|dont", "finished|completed|finish|complete"]) {
            self.push(rule(Field::Finished, Op::Is, "false"));
            return Some(2);
        }
        if self.seq(0, &["never|didnt|havent", "got|get|made|make", "through"]) {
            self.push(rule(Field::Finished, Op::Is, "false"));
            return Some(3);
        }
        if self.seq(0, &["half", "finished|listened|played"]) {
            self.push(rule(Field::Finished, Op::Is, "false"));
            return Some(2);
        }
        None
    }

    /// "the Yandhi era": an era (or, failing that, an album) by name.
    fn era(&mut self) -> Option<usize> {
        if FILLER.contains(&self.w(0)) {
            return None;
        }
        for n in (1..=6).rev() {
            if self.w(n) != "era" && self.w(n) != "eras" {
                continue;
            }
            let words: Vec<String> = self.toks[self.i..self.i + n].iter().map(|t| t.w.clone()).collect();
            let eras: Vec<String> = match self.vocab.named(Field::Era, &words.join(" ")) {
                Some(e) => vec![e],
                None => self.vocab.starting(&words, Field::Era),
            };
            if let Some((first, rest)) = eras.split_first() {
                let others = rest.iter().map(|e| rule(Field::Era, Op::Is, e.clone())).collect();
                self.push_with(rule(Field::Era, Op::Is, first.clone()), others, n);
                return Some(n + 1);
            }
            if let Some(album) = self.vocab.named(Field::Album, &words.join(" ")) {
                self.push(rule(Field::Album, Op::Is, album));
                return Some(n + 1);
            }
        }
        None
    }

    fn length(&mut self) -> Option<usize> {
        let unit_at = |p: &Self, k: usize| -> Option<f64> {
            match p.toks.get(p.i + k).map(|t| t.w.as_str()).unwrap_or("") {
                "minute" | "minutes" | "min" | "mins" | "m" => Some(1.0),
                "second" | "seconds" | "sec" | "secs" | "s" => Some(1.0 / 60.0),
                _ => None,
            }
        };
        let fmt = |v: f64| {
            let v = (v * 100.0).round() / 100.0;
            if v.fract() == 0.0 { format!("{}", v as i64) } else { format!("{v}") }
        };
        // "under 3 minutes", "longer than 5 min"
        if let Some((n, op, v)) = self.compare_at(0)
            && let Some(u) = unit_at(self, n)
        {
            // "at least 5" is 4 for plays; for minutes keep the number.
            let v = if self.seq(0, &["at", "least"]) { v + 1.0 } else if self.seq(0, &["at", "most"]) || self.seq(0, &["up", "to"]) { v - 1.0 } else { v };
            self.push(rule(Field::Duration, op, fmt(v * u)));
            let mut j = n + 1;
            if self.seq(j, &["long"]) {
                j += 1;
            }
            return Some(j);
        }
        // "3 minutes or less", "5 min or longer"
        if let Some(v) = number(self.w(0))
            && let Some(u) = unit_at(self, 1)
        {
            if self.seq(2, &["or", "less|shorter|under|fewer"]) {
                self.push(rule(Field::Duration, Op::Lt, fmt(v as f64 * u)));
                return Some(4);
            }
            if self.seq(2, &["or", "more|longer|over"]) {
                self.push(rule(Field::Duration, Op::Gt, fmt(v as f64 * u)));
                return Some(4);
            }
        }
        None
    }

    /// "50 songs", "top 20", "2 hours", "an hour of".
    fn counts(&mut self) -> Option<usize> {
        if self.seq(0, &["top"])
            && let Some(n) = number(self.w(1)).filter(|n| *n > 0)
        {
            self.sort = SortBy::MostPlayed;
            self.limit = Some(Limit { unit: LimitUnit::Songs, value: n.min(super::rules::MAX_LIMIT_SONGS) });
            return Some(if self.seq(2, &["songs|tracks"]) { 3 } else { 2 });
        }
        if self.seq(0, &["a|an|one", "hour|hr"]) {
            self.limit = Some(Limit { unit: LimitUnit::Hours, value: 1 });
            return Some(2);
        }
        let n = number(self.w(0)).filter(|n| *n > 0)?;
        if self.seq(1, &["songs|tracks|song|track"]) {
            self.limit = Some(Limit { unit: LimitUnit::Songs, value: n.min(super::rules::MAX_LIMIT_SONGS) });
            return Some(2);
        }
        if self.seq(1, &["hours|hour|hrs|hr|h"]) {
            self.limit = Some(Limit { unit: LimitUnit::Hours, value: n.min(super::rules::MAX_LIMIT_HOURS) });
            return Some(2);
        }
        if self.seq(1, &["+", "plays"]) || self.seq(1, &["plays"]) {
            let k = if self.w(1) == "+" { 3 } else { 2 };
            self.push(rule(Field::PlayCount, Op::Gt, (n - 1).to_string()));
            return Some(k);
        }
        if let Some((k, op, v)) = self.compare_at(0)
            && self.seq(k, &["plays|times"])
        {
            self.push(rule(Field::PlayCount, op, v.to_string()));
            return Some(k + 1);
        }
        None
    }

    fn rating(&mut self) -> Option<usize> {
        if self.seq(0, &["top|highly|best|highest|well", "rated"]) {
            self.push(rule(Field::Rating, Op::Gt, "3"));
            return Some(2);
        }
        let mut k = 0;
        if self.seq(0, &["rated"]) {
            k = 1;
        }
        let n = number(self.w(k)).filter(|n| (1..=5).contains(n))?;
        let star = self.seq(k + 1, &["star|stars"]);
        if !star && k == 0 {
            return None;
        }
        let r = if n == 5 { rule(Field::Rating, Op::Is, "5") } else { rule(Field::Rating, Op::Gt, (n - 1).to_string()) };
        self.push(r);
        Some(k + 1 + usize::from(star))
    }

    fn quality(&mut self) -> Option<usize> {
        if self.seq(0, &["hi|high", "res|resolution|fi|quality"]) || self.seq(0, &["cd", "quality"]) {
            self.push(rule(Field::Lossless, Op::Is, "true"));
            return Some(2);
        }
        if self.seq(0, &["family|kid|kids|radio", "friendly|safe"]) {
            self.push(rule(Field::Explicit, Op::Is, "false"));
            return Some(2);
        }
        if self.seq(0, &["greatest", "hits"]) || self.seq(0, &["best", "of"]) {
            self.push(rule(Field::ReleaseType, Op::Is, "compilations"));
            return Some(2);
        }
        if self.seq(0, &["officially", "released"]) {
            self.push(rule(Field::Unreleased, Op::Is, "false"));
            return Some(2);
        }
        if self.seq(0, &["newest|latest", "first"]) {
            self.sort = SortBy::RecentlyAdded;
            return Some(2);
        }
        if self.seq(0, &["mixed", "up"]) {
            self.sort = SortBy::Random;
            return Some(2);
        }
        None
    }

    /// "early 90s", "late 80s", "mid 2000s".
    fn decade_part(&mut self) -> Option<usize> {
        if !self.seq(0, &["early|mid|late"]) {
            return None;
        }
        let (y, _) = decade_of(self.w(1))?;
        let (a, b) = match self.w(0) {
            "early" => (y, y + 3),
            "mid" => (y + 3, y + 6),
            _ => (y + 6, y + 9),
        };
        self.push(rule(Field::Year, Op::Gt, (a - 1).to_string()));
        self.push(rule(Field::Year, Op::Lt, (b + 1).to_string()));
        Some(2)
    }

    /// Release years ("2016", "before 2000", "2010 to 2015"), or play and
    /// added years after "played" or "added".
    fn years(&mut self) -> Option<usize> {
        let mut k = 0;
        let mut cmp = "";
        for (pat, c) in [
            (&["before|until|till|pre|prior"][..], "before"),
            (&["older", "than"][..], "before"),
            (&["after|post"][..], "after"),
            (&["newer", "than"][..], "after"),
            (&["since"][..], "since"),
            (&["between|from"][..], "from"),
        ] {
            if self.seq(0, pat) {
                k = pat.len();
                cmp = c;
                if self.seq(k, &["to"]) && c == "before" {
                    k += 1;
                }
                break;
            }
        }
        let y = year_of(self.w(k))?;
        let mut n = k + 1;
        // A second year makes a range: "2010 to 2015", "2010-2015", "between 2010 and 2015".
        let mut to = None;
        if let Some(y2) = year_of(self.w(n)).filter(|y2| *y2 >= y) {
            to = Some(y2);
            n += 1;
        } else if self.seq(n, &["to|and|through|thru|until|till"])
            && let Some(y2) = year_of(self.w(n + 1)).filter(|y2| *y2 >= y)
        {
            to = Some(y2);
            n += 2;
        }
        if cmp == "from" && to.is_none() {
            // "from 2016" is just the year.
            cmp = "";
        }
        if self.ctx == Ctx::Release {
            match (cmp, to) {
                (_, Some(y2)) => {
                    self.push(rule(Field::Year, Op::Gt, (y - 1).to_string()));
                    self.push(rule(Field::Year, Op::Lt, (y2 + 1).to_string()));
                }
                ("before", _) => {
                    self.push(rule(Field::Year, Op::Lt, y.to_string()));
                }
                ("after", _) => {
                    self.push(rule(Field::Year, Op::Gt, y.to_string()));
                }
                ("since", _) => {
                    self.push(rule(Field::Year, Op::Gt, (y - 1).to_string()));
                }
                _ => {
                    self.push(rule(Field::Year, Op::Is, y.to_string()));
                }
            }
            return Some(n);
        }
        let f = self.time_field();
        let start = (y, 1, 1);
        let range = match (cmp, to) {
            (_, Some(y2)) => (start, (y2, 12, 31)),
            ("since" | "after", _) => (if cmp == "after" { (y + 1, 1, 1) } else { start }, self.today),
            ("before", _) => ((1900, 1, 1), (y - 1, 12, 31)),
            _ => (start, (y, 12, 31)),
        };
        self.range(f, range);
        Some(n)
    }

    /// Calendar periods: "last summer", "this year", "last month", "June",
    /// "summer 2019", "yesterday". Plays unless "added" came before; "from
    /// last year" alone is a release year.
    fn periods(&mut self) -> Option<usize> {
        let today = self.today;
        let prev_from = self.i > 0 && self.toks[self.i - 1].w == "from" && self.ctx == Ctx::Release;
        let (n, range): (usize, (Day, Day)) = if self.seq(0, &["last|past|previous", "spring|summer|fall|autumn|winter"]) {
            (2, last_season(self.w(1), today)?)
        } else if self.seq(0, &["this", "spring|summer|fall|autumn|winter"]) {
            (2, this_season(self.w(1), today)?)
        } else if self.seq(0, &["spring|summer|fall|autumn|winter"]) && (year_of(self.w(1)).is_some() || (self.w(1) == "of" && year_of(self.w(2)).is_some())) {
            let k = if self.w(1) == "of" { 2 } else { 1 };
            (k + 1, season(self.w(0), year_of(self.w(k))?)?)
        } else if self.seq(0, &["last|previous", "year"]) {
            if prev_from {
                self.push(rule(Field::Year, Op::Is, (today.0 - 1).to_string()));
                return Some(2);
            }
            (2, ((today.0 - 1, 1, 1), (today.0 - 1, 12, 31)))
        } else if self.seq(0, &["this", "year"]) {
            if prev_from {
                self.push(rule(Field::Year, Op::Is, today.0.to_string()));
                return Some(2);
            }
            (2, ((today.0, 1, 1), today))
        } else if self.seq(0, &["last|previous", "month"]) {
            let (y, m) = if today.1 == 1 { (today.0 - 1, 12) } else { (today.0, today.1 - 1) };
            (2, ((y, m, 1), month_end(y, m)))
        } else if self.seq(0, &["this", "month"]) {
            (2, ((today.0, today.1, 1), today))
        } else if self.seq(0, &["last|previous", "week"]) {
            // Weeks start on Monday; 1970-01-01 was a Thursday.
            let monday = days(today) - (days(today) + 3).rem_euclid(7);
            (2, (day(monday - 7), day(monday - 1)))
        } else if self.seq(0, &["this", "week"]) {
            let monday = days(today) - (days(today) + 3).rem_euclid(7);
            (2, (day(monday), today))
        } else if self.seq(0, &["yesterday"]) {
            let y = day(days(today) - 1);
            (1, (y, y))
        } else if self.seq(0, &["today|tonight"]) {
            (1, (today, today))
        } else {
            let m = month_of(self.w(0))?;
            // A month alone needs a time word before it ("played in June").
            let y = year_of(self.w(1));
            if y.is_none() && self.ctx == Ctx::Release {
                return None;
            }
            let year = y.unwrap_or(if m <= today.1 { today.0 } else { today.0 - 1 });
            (1 + usize::from(y.is_some()), ((year, m, 1), month_end(year, m)))
        };
        let f = self.time_field();
        self.range(f, range);
        Some(n)
    }

    fn sorts(&mut self) -> Option<usize> {
        if self.seq(0, &["sorted|ordered", "by"]) && self.seq(2, &["plays|popularity"]) {
            self.sort = SortBy::MostPlayed;
            return Some(3);
        }
        None
    }

    fn finish(mut self, text: &str) -> Described {
        let mut match_all = true;
        // Two values for one field: "rock or jazz" means either.
        let fields: HashSet<Field> = self.rules.iter().map(|r| r.field).collect();
        let mut doubled: Vec<Field> = Vec::new();
        let mut fields_sorted: Vec<Field> = fields.iter().copied().collect();
        fields_sorted.sort_by_key(|f| f.label());
        for f in &fields_sorted {
            let pos = self.rules.iter().filter(|r| r.field == *f && r.op == Op::Is && r.field.kind() != super::rules::Kind::Bool).count();
            if pos > 1 {
                doubled.push(*f);
            }
        }
        let mut groups: Vec<RuleGroup> = Vec::new();
        // "Rock or jazz, lossless": either genre, and lossless. The two go in a group of their
        // own.
        let grouped = doubled.len() == 1 && self.or_seen && fields.len() > 1;
        if fields.len() == 1 && (doubled.len() == 1 || self.or_seen) && self.rules.len() > 1 {
            match_all = false;
        } else if grouped {
            let f = doubled[0];
            let either = |r: &Rule| r.field == f && r.op == Op::Is;
            // Where each rule goes: (in the group, its place).
            let (mut kept, mut moved) = (0, 0);
            let mut place: Vec<(bool, usize)> = Vec::new();
            for r in &self.rules {
                if either(r) {
                    place.push((true, moved));
                    moved += 1;
                } else {
                    place.push((false, kept));
                    kept += 1;
                }
            }
            for c in &mut self.choices {
                if let Some(&(in_group, i)) = place.get(c.index) {
                    c.index = i;
                    c.group = in_group.then_some(0);
                }
            }
            let (one, rest): (Vec<Rule>, Vec<Rule>) = std::mem::take(&mut self.rules).into_iter().partition(either);
            self.rules = rest;
            groups.push(RuleGroup { match_all: false, rules: one });
        } else if let Some(f) = doubled.first() {
            let names: Vec<&str> = self.rules.iter().filter(|r| r.field == *f && r.op == Op::Is).map(|r| r.value.as_str()).collect();
            self.notes.push(format!(
                "Songs have to match every rule, so these need {} {}. Choose \"any of these rules\" to get either.",
                f.label().to_lowercase(),
                names.join(" and ")
            ));
        } else if self.or_seen && self.rules.len() > 1 && fields.len() > 1 {
            self.notes.push("\"Or\" works across all the rules at once: choose \"any of these rules\" to get songs that match any one.".into());
        }
        // Unknown words, as typed, runs joined.
        let mut unknown: Vec<String> = Vec::new();
        let mut last: Option<usize> = None;
        for &k in &self.unknown {
            let raw = self.toks[k].raw.clone();
            match (last, unknown.last_mut()) {
                (Some(l), Some(u)) if l + 1 == k => {
                    u.push(' ');
                    u.push_str(&raw);
                }
                _ => unknown.push(raw),
            }
            last = Some(k);
        }
        let title: String = {
            let t = text.trim().trim_end_matches(['.', '!', '?', ',']).trim();
            let mut c = t.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).take(100).collect(),
                None => String::new(),
            }
        };
        Described {
            title,
            rules: SmartRules { match_all, rules: self.rules, groups, limit: self.limit, sort: self.sort, ..Default::default() },
            unknown,
            choices: self.choices,
            notes: self.notes,
        }
    }
}

/// Turn a description into rules. `today` is the local calendar day.
pub fn describe(text: &str, vocab: &Vocabulary, today: Day) -> Described {
    let text: String = text.chars().take(MAX_TEXT).collect();
    let mut p = Parser {
        toks: tokens(&text),
        i: 0,
        vocab,
        today,
        rules: Vec::new(),
        choices: Vec::new(),
        unknown: Vec::new(),
        notes: Vec::new(),
        negate: false,
        ctx: Ctx::Release,
        or_seen: false,
        sort: SortBy::None,
        limit: None,
    };
    p.run();
    p.finish(&text)
}

/// The local calendar day for unix seconds and a time zone (minutes east of UTC).
pub fn local_day(now: i64, tz_min: i32) -> Day {
    day((now + tz_min as i64 * 60).div_euclid(86_400))
}
