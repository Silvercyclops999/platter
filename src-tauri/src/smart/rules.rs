//! The rule set a smart playlist is made from, as the editor shows it.
//!
//! The same rules either become a Plex smart playlist (see [`super::plexuri`])
//! or a Platter one evaluated on this computer (see [`super::eval`]).

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const MAX_RULES: usize = 30;
pub const MAX_LIMIT_SONGS: u32 = 10_000;
pub const MAX_LIMIT_HOURS: u32 = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    Genre,
    Style,
    Mood,
    Artist,
    Album,
    Year,
    Decade,
    /// When the album was added to the library.
    AddedAt,
    PlayCount,
    LastPlayed,
    /// Played on this computer (the local listen log).
    PlayedHere,
    /// Stars, 1 to 5.
    Rating,
    Liked,
    /// Minutes.
    Duration,
    /// The release type section ("albums", "singlesEps", "live"...).
    ReleaseType,
    /// From the file tags.
    Explicit,
    /// The file tags' version ("remaster", "demo"...).
    Version,
    /// The file tags' era (ARCHIVE_ERA).
    Era,
    /// Demos, leaks and other unreleased versions.
    Unreleased,
    /// FLAC, ALAC, WAV and other lossless files.
    Lossless,
    /// Times skipped (listen history).
    Skips,
    /// Every song on its album has been played.
    Finished,
    /// Any play in the listen history (not just the newest).
    Played,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    Is,
    IsNot,
    Contains,
    NotContains,
    Gt,
    Lt,
    InLast,
    NotInLast,
    /// Between two local calendar days, both included ("2026-06-01..2026-08-31").
    Between,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Tag,
    Text,
    Number,
    Decade,
    Date,
    Bool,
    Choice,
}

impl Field {
    pub fn kind(self) -> Kind {
        match self {
            Field::Genre | Field::Style | Field::Mood => Kind::Tag,
            Field::Artist | Field::Album | Field::Version | Field::Era => Kind::Text,
            Field::Year | Field::PlayCount | Field::Rating | Field::Duration | Field::Skips => Kind::Number,
            Field::Decade => Kind::Decade,
            Field::AddedAt | Field::LastPlayed | Field::PlayedHere | Field::Played => Kind::Date,
            Field::Liked | Field::Explicit | Field::Unreleased | Field::Lossless | Field::Finished => Kind::Bool,
            Field::ReleaseType => Kind::Choice,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Field::Genre => "Genre",
            Field::Style => "Style",
            Field::Mood => "Mood",
            Field::Artist => "Artist",
            Field::Album => "Album",
            Field::Year => "Year",
            Field::Decade => "Decade",
            Field::AddedAt => "Added",
            Field::PlayCount => "Plays",
            Field::LastPlayed => "Last played",
            Field::PlayedHere => "Played on this computer",
            Field::Rating => "Rating",
            Field::Liked => "Liked",
            Field::Duration => "Length",
            Field::ReleaseType => "Release type",
            Field::Explicit => "Explicit",
            Field::Version => "Version",
            Field::Era => "Era",
            Field::Unreleased => "Unreleased",
            Field::Lossless => "Lossless",
            Field::Skips => "Skips",
            Field::Finished => "Album finished",
            Field::Played => "Played",
        }
    }
}

impl Field {
    /// Filled on this computer only (Plex has nothing to filter it on).
    pub fn local_only(self) -> bool {
        matches!(
            self,
            Field::PlayedHere
                | Field::Duration
                | Field::ReleaseType
                | Field::Explicit
                | Field::Version
                | Field::Era
                | Field::Unreleased
                | Field::Lossless
                | Field::Skips
                | Field::Finished
                | Field::Played
        )
    }
}

impl Kind {
    pub fn ops(self) -> &'static [Op] {
        match self {
            Kind::Tag | Kind::Decade | Kind::Choice => &[Op::Is, Op::IsNot],
            Kind::Text => &[Op::Is, Op::IsNot, Op::Contains, Op::NotContains],
            Kind::Number => &[Op::Is, Op::IsNot, Op::Gt, Op::Lt],
            Kind::Date => &[Op::InLast, Op::NotInLast, Op::Between],
            Kind::Bool => &[Op::Is],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub field: Field,
    pub op: Op,
    pub value: String,
    /// The Plex field it was read from, kept so saving it back doesn't
    /// change what it means (`track.genre` stays `track.genre`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plex_field: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LimitUnit {
    Songs,
    Hours,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub unit: LimitUnit,
    pub value: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortBy {
    /// Library order (artist, album, track).
    #[default]
    None,
    Random,
    MostPlayed,
    LeastPlayed,
    RecentlyAdded,
    RecentlyPlayed,
    Rating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Update {
    /// Always up to date.
    #[default]
    Live,
    Daily,
    Weekly,
}

impl Update {
    /// Seconds between updates (None: live).
    pub fn period_s(self) -> Option<i64> {
        match self {
            Update::Live => None,
            Update::Daily => Some(24 * 3600),
            Update::Weekly => Some(7 * 24 * 3600),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmartRules {
    /// Every rule must match (otherwise any one).
    pub match_all: bool,
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub limit: Option<Limit>,
    #[serde(default)]
    pub sort: SortBy,
    #[serde(default)]
    pub update: Update,
    /// Plex options the editor doesn't show, kept as they are
    /// (one copy per song, other sorts).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<(String, String)>,
}

impl Default for SmartRules {
    fn default() -> Self {
        Self { match_all: true, rules: Vec::new(), limit: None, sort: SortBy::None, update: Update::Live, extra: Vec::new() }
    }
}

/// A relative time like "30d", "2w", "6m" or "1y", in seconds.
pub fn parse_span(v: &str) -> Option<i64> {
    let v = v.trim().to_ascii_lowercase();
    let split = v.find(|c: char| !c.is_ascii_digit())?;
    let (n, unit) = v.split_at(split);
    let n: i64 = n.parse().ok().filter(|n| *n > 0 && *n <= 10_000)?;
    let day = 24 * 3600;
    let s = match unit {
        "d" => day,
        "w" => 7 * day,
        "m" | "mon" => 30 * day,
        "y" => 365 * day,
        _ => return None,
    };
    Some(n * s)
}

/// A calendar day.
pub type Day = (i32, u32, u32);

/// "2026-06-01..2026-08-31": two local calendar days, the first not after
/// the second.
pub fn parse_range(v: &str) -> Option<(Day, Day)> {
    let (a, b) = v.trim().split_once("..")?;
    let day = |s: &str| {
        let d = crate::archive::eras::parse_date(s)?;
        Some((d.year, d.month? as u32, d.day? as u32))
    };
    let (a, b) = (day(a)?, day(b)?);
    (a <= b).then_some((a, b))
}

/// The range value for two days.
pub fn range_value(a: Day, b: Day) -> String {
    format!("{:04}-{:02}-{:02}..{:04}-{:02}-{:02}", a.0, a.1, a.2, b.0, b.1, b.2)
}

pub const RELEASE_TYPES: &[&str] = &["albums", "singlesEps", "live", "compilations", "soundtracks", "remixes", "demos", "mixtapes"];

fn bad(msg: impl Into<String>) -> AppError {
    AppError::Invalid(msg.into())
}

impl Rule {
    pub fn check(&self) -> AppResult<()> {
        let kind = self.field.kind();
        if !kind.ops().contains(&self.op) {
            return Err(bad(format!("{} can't use that comparison.", self.field.label())));
        }
        let v = self.value.trim();
        if v.chars().count() > 200 {
            return Err(bad("Rule values can be up to 200 characters."));
        }
        let ok = match kind {
            Kind::Tag | Kind::Text => !v.is_empty(),
            Kind::Number => v.parse::<f64>().is_ok_and(|n| n >= 0.0 && (self.field != Field::Rating || n <= 5.0)),
            Kind::Decade => v.parse::<i32>().is_ok_and(|y| y % 10 == 0 && (1000..=2990).contains(&y)),
            Kind::Date if self.op == Op::Between => parse_range(v).is_some(),
            Kind::Date => parse_span(v).is_some(),
            Kind::Bool => v == "true" || v == "false",
            Kind::Choice => RELEASE_TYPES.contains(&v),
        };
        if !ok {
            return Err(bad(format!("Check the value for {}.", self.field.label())));
        }
        Ok(())
    }
}

impl SmartRules {
    pub fn check(&self) -> AppResult<()> {
        if self.rules.len() > MAX_RULES {
            return Err(bad(format!("A smart playlist can have up to {MAX_RULES} rules.")));
        }
        for r in &self.rules {
            r.check()?;
        }
        if let Some(l) = self.limit {
            let max = match l.unit {
                LimitUnit::Songs => MAX_LIMIT_SONGS,
                LimitUnit::Hours => MAX_LIMIT_HOURS,
            };
            if l.value == 0 || l.value > max {
                return Err(bad("Check the limit."));
            }
        }
        Ok(())
    }

    /// Why Plex can't hold these rules (empty: it can).
    pub fn plex_gaps(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut add = |s: String| {
            if !out.contains(&s) {
                out.push(s);
            }
        };
        for r in &self.rules {
            if r.field.local_only() {
                add(format!("{} isn't something Plex can filter on", r.field.label()));
            } else if r.op == Op::Between {
                add("Plex can't filter between two dates".into());
            }
        }
        if matches!(self.limit, Some(Limit { unit: LimitUnit::Hours, .. })) {
            add("Plex limits by number of songs, not hours".into());
        }
        if self.update != Update::Live {
            add("Plex smart playlists are always up to date".into());
        }
        out
    }
}
