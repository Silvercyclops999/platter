//! Credits and release facts a file carries, for the album's sleeve notes.
//!
//! Vorbis comments and MP4 freeform atoms name each credit (COMPOSER,
//! PRODUCER, PERFORMER "Name (instrument)", ...). ID3 has its own frames:
//! TCOM composer, TEXT lyricist, TPUB label, TDOR original date, and the
//! paired lists TIPL (role, name) and TMCL (instrument, name). Release facts
//! are the label, catalogue number, barcode, release country and original
//! date as MusicBrainz Picard and most taggers write them.

use serde::{Deserialize, Serialize};

/// Most credits kept from one file.
pub const MAX_CREDITS: usize = 64;

/// One person a file credits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagCredit {
    /// One of the sleeve roles (see `crate::sleeve::ROLES`).
    pub role: String,
    pub name: String,
    /// The instrument for a performer ("guitar").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// What a file says for the sleeve notes. Empty for most files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TagSleeve {
    pub credits: Vec<TagCredit>,
    pub label: Option<String>,
    pub catalogue: Option<String>,
    pub barcode: Option<String>,
    pub country: Option<String>,
    pub original_date: Option<String>,
}

/// Tag names read for the sleeve (upper case; ID3 frames map onto these).
pub const KEYS: &[&str] = &[
    "COMPOSER",
    "LYRICIST",
    "WRITER",
    "SONGWRITER",
    "PRODUCER",
    "ENGINEER",
    "MIXER",
    "MIXINGENGINEER",
    "MIXING ENGINEER",
    "MASTERINGENGINEER",
    "MASTERING ENGINEER",
    "MASTERING",
    "ARRANGER",
    "CONDUCTOR",
    "PERFORMER",
    "LABEL",
    "ORGANIZATION",
    "CATALOGNUMBER",
    "CATALOGUENUMBER",
    "BARCODE",
    "UPC",
    "RELEASECOUNTRY",
    "MUSICBRAINZ ALBUM RELEASE COUNTRY",
    "ORIGINALDATE",
    "ORIGINALYEAR",
    "ORIGINAL YEAR",
];

/// The sleeve role a credit tag names.
pub fn role_for_key(key: &str) -> Option<&'static str> {
    Some(match key {
        "COMPOSER" => "composer",
        "LYRICIST" => "lyricist",
        "WRITER" | "SONGWRITER" => "writer",
        "PRODUCER" => "producer",
        "ENGINEER" => "engineer",
        "MIXER" | "MIXINGENGINEER" | "MIXING ENGINEER" => "mixer",
        "MASTERINGENGINEER" | "MASTERING ENGINEER" | "MASTERING" => "mastering",
        "ARRANGER" => "arranger",
        "CONDUCTOR" => "conductor",
        "PERFORMER" => "performer",
        _ => return None,
    })
}

/// The sleeve role for an ID3 TIPL involvement ("producer", "mix", ...).
pub fn role_for_involvement(what: &str) -> Option<&'static str> {
    let w = what.trim().to_ascii_lowercase();
    Some(match w.as_str() {
        "producer" | "co-producer" => "producer",
        "engineer" | "recording engineer" | "recording" => "engineer",
        "mix" | "mixer" | "mixing" => "mixer",
        "mastering" | "mastering engineer" => "mastering",
        "arranger" => "arranger",
        "composer" => "composer",
        "lyricist" => "lyricist",
        "writer" => "writer",
        "conductor" => "conductor",
        _ => return None,
    })
}

/// Splits a value holding several names ("A; B", "A / B"). A plain "/"
/// stays, so "AC/DC" is one name.
pub fn split_names(v: &str) -> impl Iterator<Item = &str> {
    v.split(';').flat_map(|p| p.split(" / ")).map(str::trim).filter(|p| !p.is_empty())
}

/// "Name (instrument)" as written in a PERFORMER tag.
pub fn performer(v: &str) -> (String, Option<String>) {
    let v = v.trim();
    if v.ends_with(')')
        && let Some(i) = v.rfind(" (")
    {
        let inst = v[i + 2..v.len() - 1].trim();
        let name = v[..i].trim();
        if !name.is_empty() && !inst.is_empty() {
            return (name.to_owned(), Some(inst.to_lowercase()));
        }
    }
    (v.to_owned(), None)
}

impl TagSleeve {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Adds a credit once (same role, name and instrument).
    pub fn credit(&mut self, role: &str, name: &str, detail: Option<String>) {
        let name = name.trim();
        if name.is_empty() || self.credits.len() >= MAX_CREDITS {
            return;
        }
        let detail = detail.filter(|d| !d.is_empty());
        if self.credits.iter().any(|c| c.role == role && c.name.eq_ignore_ascii_case(name) && c.detail == detail) {
            return;
        }
        self.credits.push(TagCredit { role: role.to_owned(), name: name.to_owned(), detail });
    }

    /// Records one tag (upper-case key). Returns whether it was a sleeve tag.
    pub fn take(&mut self, key: &str, v: &str) -> bool {
        let first = |slot: &mut Option<String>| {
            if slot.is_none() {
                *slot = Some(v.trim().to_owned());
            }
        };
        if let Some(role) = role_for_key(key) {
            for n in split_names(v) {
                if role == "performer" {
                    let (name, inst) = performer(n);
                    self.credit(role, &name, inst);
                } else {
                    self.credit(role, n, None);
                }
            }
            return true;
        }
        match key {
            "LABEL" | "ORGANIZATION" => first(&mut self.label),
            "CATALOGNUMBER" | "CATALOGUENUMBER" => first(&mut self.catalogue),
            "BARCODE" | "UPC" => first(&mut self.barcode),
            "RELEASECOUNTRY" | "MUSICBRAINZ ALBUM RELEASE COUNTRY" => first(&mut self.country),
            "ORIGINALDATE" | "ORIGINALYEAR" | "ORIGINAL YEAR" => first(&mut self.original_date),
            _ => return false,
        }
        true
    }

    /// An ID3 paired list: TIPL (involvement, name) or, with `musicians`,
    /// TMCL (instrument, name).
    pub fn pairs(&mut self, values: &[String], musicians: bool) {
        for pair in values.chunks(2) {
            let [what, who] = pair else { break };
            if musicians {
                for n in split_names(who) {
                    self.credit("performer", n, Some(what.trim().to_lowercase()));
                }
            } else if let Some(role) = role_for_involvement(what) {
                for n in split_names(who) {
                    self.credit(role, n, None);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credit_tags_split_and_keep_each_person_once() {
        let mut s = TagSleeve::default();
        assert!(s.take("PRODUCER", "Ada Moss; Bo Kettle"));
        assert!(s.take("PRODUCER", "ada moss"));
        assert!(s.take("COMPOSER", "AC/DC / Rue Ellis"));
        assert!(s.take("PERFORMER", "Cy Hart (Guitar)"));
        assert!(!s.take("TITLE", "x"));
        let got: Vec<(&str, &str, Option<&str>)> =
            s.credits.iter().map(|c| (c.role.as_str(), c.name.as_str(), c.detail.as_deref())).collect();
        assert_eq!(
            got,
            [
                ("producer", "Ada Moss", None),
                ("producer", "Bo Kettle", None),
                ("composer", "AC/DC", None),
                ("composer", "Rue Ellis", None),
                ("performer", "Cy Hart", Some("guitar")),
            ]
        );
    }

    #[test]
    fn release_facts_first_value_wins() {
        let mut s = TagSleeve::default();
        s.take("LABEL", "Low Tide");
        s.take("ORGANIZATION", "Other");
        s.take("CATALOGNUMBER", "LT-001");
        s.take("BARCODE", "0123456789012");
        s.take("RELEASECOUNTRY", "GB");
        s.take("ORIGINALDATE", "1997-05-21");
        assert_eq!(s.label.as_deref(), Some("Low Tide"));
        assert_eq!(s.catalogue.as_deref(), Some("LT-001"));
        assert_eq!(s.barcode.as_deref(), Some("0123456789012"));
        assert_eq!(s.country.as_deref(), Some("GB"));
        assert_eq!(s.original_date.as_deref(), Some("1997-05-21"));
    }

    #[test]
    fn id3_pairs_become_roles_and_instruments() {
        let mut s = TagSleeve::default();
        let v = |xs: &[&str]| xs.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        s.pairs(&v(&["producer", "Ada Moss", "mix", "Dee Lane", "artwork", "Nobody", "engineer"]), false);
        s.pairs(&v(&["Bass", "Eli Stone"]), true);
        let got: Vec<(&str, &str)> = s.credits.iter().map(|c| (c.role.as_str(), c.name.as_str())).collect();
        assert_eq!(got, [("producer", "Ada Moss"), ("mixer", "Dee Lane"), ("performer", "Eli Stone")]);
        assert_eq!(s.credits[2].detail.as_deref(), Some("bass"));
    }

    #[test]
    fn credits_are_capped() {
        let mut s = TagSleeve::default();
        for i in 0..100 {
            s.credit("writer", &format!("P{i}"), None);
        }
        assert_eq!(s.credits.len(), MAX_CREDITS);
    }
}
