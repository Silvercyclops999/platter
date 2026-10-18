//! MusicBrainz details: an artist's members and relationships, and a
//! release's dates and credits (producer, writer, composer, lyricist,
//! featured artists, mixing, engineering, mastering, players and their
//! instruments, arrangers, conductors), plus what the sleeve notes show:
//! catalogue number, country, barcode, format, studios, recording dates and
//! the release's annotation. Pure parsing of the lookup answers, so the rules
//! are easy to test against fixtures.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{fold_name, is_mbid};

/// Credit roles kept, in the order a person's roles are listed.
pub const ROLES: [&str; 11] =
    ["producer", "writer", "composer", "lyricist", "featured", "mixer", "engineer", "mastering", "performer", "arranger", "conductor"];

/// Raised when [`parse_release`] starts keeping something new, so releases
/// cached by an older build are looked up again.
/// 2: catalogue number, country, barcode, formats, studios, recording dates,
/// annotation and players with their instruments.
pub const PARSE_VERSION: u32 = 2;

/// Longest annotation kept (characters).
pub const ANNOTATION_MAX: usize = 2000;

/// What MusicBrainz says about an artist (cached per artist).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MbArtist {
    pub mbid: String,
    pub members: Vec<MbMember>,
    pub related: Vec<MbRelated>,
    /// "1991–present", "1965–1970".
    pub life_span: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MbMember {
    pub name: String,
    pub mbid: Option<String>,
    /// "1991–present", "1991–2004", or None when MusicBrainz has no dates.
    pub years: Option<String>,
    /// Still a member.
    pub current: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MbRelated {
    pub name: String,
    pub mbid: Option<String>,
    /// "Member of", "Subgroup", "Collaboration", "Also known as", …
    pub relation: String,
}

/// A person credited on a release or a track, with their roles (see [`ROLES`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MbCredit {
    pub name: String,
    pub mbid: Option<String>,
    pub roles: Vec<String>,
    /// What a performer plays or sings ("guitar", "lead vocals").
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub instruments: Vec<String>,
}

/// A studio a release or recording names: where it was recorded, mixed or mastered.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MbPlace {
    pub name: String,
    /// "recorded", "mixed" or "mastered".
    pub role: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MbTrack {
    pub disc: u32,
    pub position: u32,
    pub title: String,
    pub credits: Vec<MbCredit>,
    /// When it was recorded: "1996-11" or a span "1996-11/1997-02".
    pub recorded: Option<String>,
    pub places: Vec<MbPlace>,
}

/// A release, as the album page needs it (cached per album).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MbRelease {
    pub release_id: String,
    pub release_group_id: Option<String>,
    pub title: String,
    /// This edition's date ("1997-06-16", "1997-06" or "1997").
    pub date: Option<String>,
    /// The first release of the release group (the original date).
    pub first_release_date: Option<String>,
    pub label: Option<String>,
    /// Credits on the release itself.
    pub credits: Vec<MbCredit>,
    pub tracks: Vec<MbTrack>,
    pub catalogue: Option<String>,
    /// ISO country code ("GB", "XW" for worldwide).
    pub country: Option<String>,
    pub barcode: Option<String>,
    /// This edition's name ("2011 remaster", "deluxe edition").
    pub disambiguation: Option<String>,
    pub status: Option<String>,
    pub packaging: Option<String>,
    /// Each medium's format in order ("CD", "Vinyl").
    pub formats: Vec<String>,
    /// Studios named on the release itself.
    pub places: Vec<MbPlace>,
    /// The release's annotation, as plain text.
    pub annotation: Option<String>,
    /// [`PARSE_VERSION`] of the build that read it (0: before versions).
    pub parsed: u32,
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|x| !x.is_empty())
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn year(d: Option<&str>) -> Option<&str> {
    d.and_then(|d| d.get(..4)).filter(|y| y.bytes().all(|b| b.is_ascii_digit()))
}

/// "1991–present", "1991–2004", "–2004" or None.
fn span(begin: Option<&str>, end: Option<&str>, ended: bool) -> Option<String> {
    match (year(begin), year(end)) {
        (Some(b), Some(e)) if b == e => Some(b.to_owned()),
        (Some(b), Some(e)) => Some(format!("{b}–{e}")),
        (Some(b), None) if ended => Some(b.to_owned()),
        (Some(b), None) => Some(format!("{b}–present")),
        (None, Some(e)) => Some(format!("–{e}")),
        (None, None) => None,
    }
}

/// The artist on the other end of a relation: (name, mbid).
fn target_artist(r: &Value) -> Option<(String, Option<String>)> {
    let a = r.get("artist")?;
    let name = s(a, "name")?.to_owned();
    Some((name, s(a, "id").filter(|i| is_mbid(i)).map(str::to_owned)))
}

/// The relation label for one artist-artist relation, from this artist's side.
fn relation_label(kind: &str, forward: bool) -> Option<&'static str> {
    Some(match (kind, forward) {
        ("member of band", true) => "Member of",
        ("subgroup", true) => "Subgroup",
        ("subgroup", false) => "Subgroup of",
        ("collaboration", true) => "Collaboration",
        ("collaboration", false) => "Collaborators",
        ("is person", true) => "Also performs as",
        ("is person", false) => "Real name",
        ("artist rename", true) => "Renamed to",
        ("artist rename", false) => "Formerly",
        ("founder", true) => "Founded",
        ("founder", false) => "Founded by",
        _ => return None,
    })
}

/// Members and relationships from an artist lookup (`inc=artist-rels`).
/// A member listed once per instrument comes out once, with the widest
/// years; current members first, in MusicBrainz order otherwise.
pub fn parse_artist(v: &Value) -> Option<MbArtist> {
    let mbid = s(v, "id").filter(|i| is_mbid(i))?.to_owned();
    let life = v.get("life-span");
    let life_span = life.and_then(|l| {
        span(s(l, "begin"), s(l, "end"), l.get("ended").and_then(Value::as_bool).unwrap_or(false))
    });
    struct M {
        name: String,
        mbid: Option<String>,
        begin: Option<String>,
        end: Option<String>,
        current: bool,
    }
    let mut members: Vec<M> = Vec::new();
    let mut related: Vec<MbRelated> = Vec::new();
    for r in arr(v, "relations") {
        if s(r, "target-type") != Some("artist") {
            continue;
        }
        let Some(kind) = s(r, "type") else { continue };
        let forward = s(r, "direction") != Some("backward");
        let Some((name, id)) = target_artist(r) else { continue };
        if kind == "member of band" && !forward {
            let ended = r.get("ended").and_then(Value::as_bool).unwrap_or(false);
            let (b, e) = (s(r, "begin").map(str::to_owned), s(r, "end").map(str::to_owned));
            let key = id.clone().unwrap_or_else(|| fold_name(&name));
            match members.iter_mut().find(|m| m.mbid.clone().unwrap_or_else(|| fold_name(&m.name)) == key) {
                Some(m) => {
                    m.current |= !ended;
                    if let Some(b) = b
                        && m.begin.as_ref().is_none_or(|x| b < *x)
                    {
                        m.begin = Some(b);
                    }
                    if let Some(e) = e
                        && m.end.as_ref().is_none_or(|x| e > *x)
                    {
                        m.end = Some(e);
                    }
                }
                None => members.push(M { name, mbid: id, begin: b, end: e, current: !ended }),
            }
            continue;
        }
        let Some(label) = relation_label(kind, forward) else { continue };
        if !related.iter().any(|x| x.name == name && x.relation == label) {
            related.push(MbRelated { name, mbid: id, relation: label.to_owned() });
        }
    }
    // A current member has no end, whatever an older line-up said.
    let mut out: Vec<MbMember> = members
        .into_iter()
        .map(|m| MbMember {
            years: span(m.begin.as_deref(), if m.current { None } else { m.end.as_deref() }, !m.current),
            name: m.name,
            mbid: m.mbid,
            current: m.current,
        })
        .collect();
    out.sort_by_key(|m| !m.current);
    Some(MbArtist { mbid, members: out, related, life_span })
}

/// The kept role for a relation type, if any. Assistants aren't credited.
fn role_for(kind: &str, attributes: &[Value]) -> Option<&'static str> {
    if attributes.iter().any(|a| a.as_str() == Some("assistant")) {
        return None;
    }
    Some(match kind {
        "instrument" | "vocal" | "performer" | "programming" => "performer",
        "arranger" | "instrument arranger" | "vocal arranger" | "orchestrator" => "arranger",
        "conductor" => "conductor",
        "producer" => "producer",
        "writer" => "writer",
        "composer" => "composer",
        "lyricist" => "lyricist",
        "librettist" => "lyricist",
        "mix" => "mixer",
        "engineer" | "recording" | "audio" | "sound" => "engineer",
        "mastering" => "mastering",
        _ => return None,
    })
}

/// Attributes that describe a performance rather than what was played.
const NOT_INSTRUMENTS: &[&str] = &["guest", "additional", "solo", "minor", "co", "executive", "associate", "instrument", "vocal"];

/// What a performer relation says was played or sung.
fn instruments_for(kind: &str, attributes: &[Value]) -> Vec<String> {
    let mut out: Vec<String> = attributes
        .iter()
        .filter_map(Value::as_str)
        .map(|a| a.trim().to_lowercase())
        .filter(|a| !a.is_empty() && !NOT_INSTRUMENTS.contains(&a.as_str()))
        .collect();
    if out.is_empty() {
        match kind {
            "vocal" => out.push("vocals".into()),
            "programming" => out.push("programming".into()),
            _ => {}
        }
    }
    out
}

/// Add a role for a person, keeping each person once.
fn add(credits: &mut Vec<MbCredit>, name: &str, mbid: Option<&str>, role: &str) {
    add_with(credits, name, mbid, role, &[]);
}

/// [`add`] with the instruments a performer played.
fn add_with(credits: &mut Vec<MbCredit>, name: &str, mbid: Option<&str>, role: &str, instruments: &[String]) {
    let key = fold_name(name);
    let found = credits.iter_mut().find(|c| match (&c.mbid, mbid) {
        (Some(a), Some(b)) => a == b,
        _ => fold_name(&c.name) == key,
    });
    let c = match found {
        Some(c) => c,
        None => {
            credits.push(MbCredit { name: name.to_owned(), mbid: mbid.map(str::to_owned), roles: vec![], instruments: vec![] });
            credits.last_mut().expect("just pushed")
        }
    };
    if !c.roles.iter().any(|r| r == role) {
        c.roles.push(role.to_owned());
    }
    for i in instruments {
        if !c.instruments.contains(i) {
            c.instruments.push(i.clone());
        }
    }
}

fn sort_roles(credits: &mut [MbCredit]) {
    for c in credits.iter_mut() {
        c.roles.sort_by_key(|r| ROLES.iter().position(|x| x == r).unwrap_or(ROLES.len()));
    }
}

/// Credits from an entity's artist relations (`relations` with target-type artist).
fn relation_credits(v: &Value, out: &mut Vec<MbCredit>) {
    for r in arr(v, "relations") {
        if s(r, "target-type") != Some("artist") {
            continue;
        }
        let kind = s(r, "type").unwrap_or_default();
        let Some(role) = role_for(kind, arr(r, "attributes")) else { continue };
        if let Some((name, id)) = target_artist(r) {
            let inst = if role == "performer" { instruments_for(kind, arr(r, "attributes")) } else { vec![] };
            add_with(out, &name, id.as_deref(), role, &inst);
        }
    }
}

/// Studios from an entity's place relations ("recorded at", "mixed at", "mastered at").
fn places(v: &Value, out: &mut Vec<MbPlace>) {
    for r in arr(v, "relations") {
        if s(r, "target-type") != Some("place") {
            continue;
        }
        let role = match s(r, "type") {
            Some("recorded at") => "recorded",
            Some("mixed at") => "mixed",
            Some("mastered at") => "mastered",
            _ => continue,
        };
        let Some(name) = r.get("place").and_then(|p| s(p, "name")) else { continue };
        if !out.iter().any(|p| p.name == name && p.role == role) {
            out.push(MbPlace { name: name.to_owned(), role: role.to_owned() });
        }
    }
}

/// When a recording was made: the earliest begin and latest end over its
/// relations (studio sessions, players), as "1996-11" or "1996-11/1997-02".
fn recorded(rec: &Value) -> Option<String> {
    let rels = arr(rec, "relations");
    let begin = rels.iter().filter_map(|r| s(r, "begin")).min();
    let end = rels.iter().filter_map(|r| s(r, "end")).max();
    match (begin, end) {
        (Some(b), Some(e)) if b != e => Some(format!("{b}/{e}")),
        (Some(b), _) => Some(b.to_owned()),
        (None, Some(e)) => Some(e.to_owned()),
        (None, None) => None,
    }
}

/// An annotation as plain text: wiki links and bold or italic marks dropped.
pub fn plain_annotation(a: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = a;
    while let Some(i) = rest.find('[') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find(']') {
            Some(j) => {
                let inner = &after[..j];
                // [url|text] keeps the text; [url] keeps the url.
                out.push_str(inner.split_once('|').map(|(_, t)| t).unwrap_or(inner));
                rest = &after[j + 1..];
            }
            None => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    let out = out.replace(&"'".repeat(3), "").replace(&"'".repeat(2), "");
    let out = out.trim();
    if out.is_empty() {
        return None;
    }
    Some(if out.chars().count() > ANNOTATION_MAX {
        let cut: String = out.chars().take(ANNOTATION_MAX).collect();
        format!("{}…", cut.trim_end())
    } else {
        out.to_owned()
    })
}

/// Whether a join phrase brings in featured artists ("feat.", "ft.", "featuring").
fn featuring(join: &str) -> bool {
    let j = join.to_ascii_lowercase();
    j.contains("feat") || j.split(|c: char| !c.is_alphanumeric()).any(|w| w == "ft")
}

/// Featured artists in an artist credit: everyone after a "feat." join.
fn featured(credit: &[Value], out: &mut Vec<MbCredit>) {
    let mut after = false;
    for c in credit {
        if after
            && let Some(name) = s(c, "name").or_else(|| c.get("artist").and_then(|a| s(a, "name")))
        {
            let id = c.get("artist").and_then(|a| s(a, "id")).filter(|i| is_mbid(i));
            add(out, name, id, "featured");
        }
        if s(c, "joinphrase").is_some_and(featuring) {
            after = true;
        }
    }
}

/// A release lookup (`inc=release-groups+artist-credits+labels+recordings+
/// artist-rels+recording-level-rels+work-rels+work-level-rels`).
pub fn parse_release(v: &Value) -> Option<MbRelease> {
    let release_id = s(v, "id").filter(|i| is_mbid(i))?.to_owned();
    let rg = v.get("release-group");
    let mut credits = Vec::new();
    relation_credits(v, &mut credits);
    sort_roles(&mut credits);
    let mut tracks = Vec::new();
    for (mi, medium) in arr(v, "media").iter().enumerate() {
        let disc = medium.get("position").and_then(Value::as_u64).unwrap_or(mi as u64 + 1) as u32;
        for (ti, t) in arr(medium, "tracks").iter().enumerate() {
            let position = t.get("position").and_then(Value::as_u64).unwrap_or(ti as u64 + 1) as u32;
            let rec = t.get("recording").unwrap_or(&Value::Null);
            let title = s(t, "title").or_else(|| s(rec, "title")).unwrap_or_default().to_owned();
            let mut c = Vec::new();
            // The track's own credit line, else the recording's.
            let line = if arr(t, "artist-credit").is_empty() { arr(rec, "artist-credit") } else { arr(t, "artist-credit") };
            featured(line, &mut c);
            relation_credits(rec, &mut c);
            for w in arr(rec, "relations").iter().filter(|r| s(r, "target-type") == Some("work")) {
                if let Some(work) = w.get("work") {
                    relation_credits(work, &mut c);
                }
            }
            sort_roles(&mut c);
            let mut ps = Vec::new();
            places(rec, &mut ps);
            tracks.push(MbTrack { disc, position, title, credits: c, recorded: recorded(rec), places: ps });
        }
    }
    Some(MbRelease {
        release_id,
        release_group_id: rg.and_then(|g| s(g, "id")).filter(|i| is_mbid(i)).map(str::to_owned),
        title: s(v, "title").unwrap_or_default().to_owned(),
        date: s(v, "date").map(str::to_owned),
        first_release_date: rg.and_then(|g| s(g, "first-release-date")).map(str::to_owned),
        label: arr(v, "label-info").iter().find_map(|l| l.get("label").and_then(|x| s(x, "name"))).map(str::to_owned),
        catalogue: arr(v, "label-info")
            .iter()
            .find_map(|l| s(l, "catalog-number"))
            .filter(|c| !c.eq_ignore_ascii_case("[none]"))
            .map(str::to_owned),
        country: s(v, "country").map(str::to_owned),
        barcode: s(v, "barcode").map(str::to_owned),
        disambiguation: s(v, "disambiguation").map(str::to_owned),
        status: s(v, "status").map(str::to_owned),
        packaging: s(v, "packaging").filter(|p| !p.eq_ignore_ascii_case("none")).map(str::to_owned),
        formats: arr(v, "media").iter().filter_map(|m| s(m, "format")).map(str::to_owned).collect(),
        places: {
            let mut ps = Vec::new();
            places(v, &mut ps);
            ps
        },
        annotation: s(v, "annotation").and_then(plain_annotation),
        parsed: PARSE_VERSION,
        credits,
        tracks,
    })
}

/// The release to read for a release group (a lookup with `inc=releases`):
/// an official one with as many tracks as the album when there is one, the
/// earliest first. None when the group has no releases.
pub fn pick_release(v: &Value, track_count: usize) -> Option<String> {
    let mut rs: Vec<&Value> = arr(v, "releases").iter().filter(|r| s(r, "id").is_some_and(is_mbid)).collect();
    // Undated releases go last.
    rs.sort_by_key(|r| s(r, "date").map(str::to_owned).unwrap_or_else(|| "9999".into()));
    let official = |r: &&Value| s(r, "status").is_none_or(|st| st.eq_ignore_ascii_case("official"));
    let count = |r: &&Value| {
        let n: u64 = arr(r, "media").iter().filter_map(|m| m.get("track-count").and_then(Value::as_u64)).sum();
        n as usize == track_count
    };
    rs.iter()
        .find(|r| official(r) && count(r))
        .or_else(|| rs.iter().find(|r| official(r)))
        .or_else(|| rs.first())
        .and_then(|r| s(r, "id"))
        .map(str::to_owned)
}

/// One credited person as the UI shows them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreditRow {
    pub name: String,
    /// A library artist with this name.
    pub artist_id: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackCredits {
    /// The library track this MusicBrainz track matched.
    pub track_id: String,
    pub credits: Vec<CreditRow>,
}

/// Where an album's MusicBrainz credits stand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CreditsState {
    /// The MusicBrainz setting is off (or Go offline is on).
    Off,
    /// Being looked up; `albumcredits://changed` follows.
    Pending,
    /// No sure match on MusicBrainz, or an unreleased album (never looked up).
    None,
    Found,
}

/// IPC `AlbumCredits`: what the album page and the now-playing credits show.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumCredits {
    pub album_id: String,
    pub state: CreditsState,
    /// The original release date ("1997-05-21"), from the release group.
    pub released: Option<String>,
    /// This edition's date, when it differs from the original.
    pub edition_date: Option<String>,
    pub label: Option<String>,
    /// Everyone credited on the album: the release's credits plus every
    /// track's (featured artists stay with their tracks).
    pub credits: Vec<CreditRow>,
    /// Per-track credits, for the tracks that matched the library.
    pub tracks: Vec<TrackCredits>,
    /// The release on musicbrainz.org.
    pub url: Option<String>,
}

impl AlbumCredits {
    pub fn empty(album_id: &str, state: CreditsState) -> Self {
        Self { album_id: album_id.to_owned(), state, released: None, edition_date: None, label: None, credits: vec![], tracks: vec![], url: None }
    }
}

/// A library track, for matching.
#[derive(Debug, Clone, PartialEq)]
pub struct LibTrack {
    pub id: String,
    pub title: String,
    pub disc: u32,
    pub number: u32,
}

/// Library track id per MusicBrainz track: by disc and position when the
/// counts agree, else by title.
pub fn match_tracks(mb: &[MbTrack], lib: &[LibTrack]) -> Vec<Option<String>> {
    use crate::share::matching::title_key;
    let by_pos = mb.len() == lib.len();
    mb.iter()
        .map(|t| {
            if by_pos && let Some(l) = lib.iter().find(|l| l.disc.max(1) == t.disc.max(1) && l.number == t.position) {
                return Some(l.id.clone());
            }
            let k = title_key(&t.title);
            lib.iter().find(|l| !k.is_empty() && title_key(&l.title) == k).map(|l| l.id.clone())
        })
        .collect()
}

fn rows(credits: &[MbCredit], library: &HashMap<String, String>) -> Vec<CreditRow> {
    credits
        .iter()
        .map(|c| CreditRow {
            name: c.name.clone(),
            artist_id: library.get(&fold_name(&c.name)).cloned(),
            roles: c.roles.clone(),
        })
        .collect()
}

/// Build the IPC answer from a cached release, the album's library tracks
/// and library artist ids by folded name.
pub fn album_credits(album_id: &str, r: &MbRelease, lib: &[LibTrack], library: &HashMap<String, String>) -> AlbumCredits {
    let ids = match_tracks(&r.tracks, lib);
    let mut all = r.credits.clone();
    for t in &r.tracks {
        for c in &t.credits {
            for role in c.roles.iter().filter(|r| *r != "featured") {
                add(&mut all, &c.name, c.mbid.as_deref(), role);
            }
        }
    }
    sort_roles(&mut all);
    // Producers first, then writers and the rest, by each person's first role.
    all.sort_by_key(|c| c.roles.first().and_then(|r| ROLES.iter().position(|x| x == r)).unwrap_or(ROLES.len()));
    let tracks = r
        .tracks
        .iter()
        .zip(ids)
        .filter_map(|(t, id)| Some(TrackCredits { track_id: id?, credits: rows(&t.credits, library) }))
        .filter(|t| !t.credits.is_empty())
        .collect();
    let released = r.first_release_date.clone().or_else(|| r.date.clone());
    AlbumCredits {
        album_id: album_id.to_owned(),
        state: CreditsState::Found,
        edition_date: r.date.clone().filter(|d| Some(d) != released.as_ref()),
        released,
        label: r.label.clone(),
        credits: rows(&all, library),
        tracks,
        url: Some(format!("https://musicbrainz.org/release/{}", r.release_id)),
    }
}

/// Every name in a release's credits (for looking up library artists).
pub fn credit_names(r: &MbRelease) -> Vec<String> {
    let mut out: Vec<String> = r.credits.iter().map(|c| c.name.clone()).collect();
    for t in &r.tracks {
        out.extend(t.credits.iter().map(|c| c.name.clone()));
    }
    out.sort();
    out.dedup();
    out
}
