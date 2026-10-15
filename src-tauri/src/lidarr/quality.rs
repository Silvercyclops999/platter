//! How good a copy is, as one comparable rank: lossy copies by bit rate,
//! lossless ones above every lossy one, by bit depth. Used to say what an
//! album is now ("MP3 320"), to pick the quality profile an upgrade asks
//! Lidarr for, and to make sure each new file really is better.
//!
//! Platter only picks among Lidarr's existing profiles. It never makes or
//! changes one.

use serde::Serialize;

use super::api::{ProfileItem, QualityProfile};

/// Comparable quality: 0 unknown, lossy by kbps (below 1000), lossless
/// 1000 + bit depth × 10 (CD quality is 1160).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Rank(pub u32);

pub const CD: Rank = Rank(1160);

const LOSSLESS: [&str; 9] = ["flac", "alac", "wav", "aiff", "aif", "pcm", "ape", "wavpack", "wv"];

impl Rank {
    pub fn lossless(bit_depth: Option<u32>) -> Rank {
        Rank(1000 + bit_depth.filter(|b| (1..=64).contains(b)).unwrap_or(16) * 10)
    }

    pub fn lossy(kbps: Option<u32>) -> Rank {
        Rank(kbps.filter(|k| *k > 0).map_or(1, |k| k.min(999)))
    }

    pub fn is_lossless(self) -> bool {
        self.0 >= 1000
    }

    /// A file's rank from its codec, bit rate (kbps) and bit depth.
    pub fn of_file(codec: Option<&str>, kbps: Option<u32>, bit_depth: Option<u32>) -> Rank {
        let c = codec.unwrap_or_default().trim().to_ascii_lowercase();
        if c.is_empty() {
            return Rank(0);
        }
        if LOSSLESS.iter().any(|l| c == *l || c.starts_with("pcm")) { Rank::lossless(bit_depth) } else { Rank::lossy(kbps) }
    }

    /// A Lidarr quality's rank from its name ("MP3-320", "FLAC 24bit",
    /// "AAC-VBR", "OGG Vorbis Q8"...). Unknown names rank 0.
    pub fn of_lidarr(name: &str) -> Rank {
        let n = name.trim().to_ascii_lowercase();
        if n.is_empty() || n == "unknown" {
            return Rank(0);
        }
        if LOSSLESS.iter().any(|l| n.starts_with(l)) {
            return Rank::lossless(Some(if n.contains("24") { 24 } else { 16 }));
        }
        if n.contains("vbr") {
            let k = if n.contains("v2") { 190 } else if n.contains("v0") || n.starts_with("mp3") { 245 } else { 200 };
            return Rank(k);
        }
        if let Some(q) = n.split("q").last().filter(|_| n.contains("vorbis")).and_then(|q| q.trim().parse::<u32>().ok()) {
            return Rank(match q {
                10 => 500,
                9 => 320,
                8 => 256,
                7 => 224,
                6 => 192,
                _ => 160,
            });
        }
        let digits: String = n.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
        match digits.parse::<u32>() {
            Ok(k) if k > 0 => Rank(k.min(999)),
            _ => Rank(128),
        }
    }

    /// Short words for the UI: "MP3 320", "FLAC", "FLAC 24-bit".
    pub fn label(self, codec: Option<&str>) -> String {
        let c = codec.map(|c| c.trim().to_ascii_uppercase()).filter(|c| !c.is_empty());
        if self.0 == 0 {
            return c.unwrap_or_else(|| "Unknown".into());
        }
        if self.is_lossless() {
            let bits = (self.0 - 1000) / 10;
            let c = c.unwrap_or_else(|| "Lossless".into());
            return if bits == 16 { c } else { format!("{c} {bits}-bit") };
        }
        let c = c.unwrap_or_else(|| "Lossy".into());
        if self.0 == 1 { c } else { format!("{c} {}", self.0) }
    }
}

/// The qualities a profile allows, as ranks.
fn allowed(items: &[ProfileItem], parent_allowed: bool, out: &mut Vec<Rank>) {
    for i in items {
        let on = i.allowed || parent_allowed;
        if let Some(q) = &i.quality
            && on
        {
            out.push(Rank::of_lidarr(&q.name));
        }
        allowed(&i.items, on && i.quality.is_none(), out);
    }
}

fn ranks_of(i: &ProfileItem) -> Vec<Rank> {
    let mut v = Vec::new();
    if let Some(q) = &i.quality {
        v.push(Rank::of_lidarr(&q.name));
    }
    for c in &i.items {
        v.extend(ranks_of(c));
    }
    v
}

/// The lowest quality that meets a profile's cutoff (a group's cutoff is met
/// by any of its qualities).
pub fn cutoff_rank(p: &QualityProfile) -> Option<Rank> {
    let cutoff = p.cutoff?;
    fn find(items: &[ProfileItem], id: i64) -> Option<&ProfileItem> {
        for i in items {
            let hit = if i.quality.is_some() && i.items.is_empty() { i.quality.as_ref().is_some_and(|q| q.id == id) } else { i.id == Some(id) };
            if hit {
                return Some(i);
            }
            if let Some(f) = find(&i.items, id) {
                return Some(f);
            }
        }
        None
    }
    find(&p.items, cutoff).and_then(|i| ranks_of(i).into_iter().min())
}

/// The profile an upgrade uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileChoice {
    pub id: i64,
    pub name: String,
    /// Lidarr stops looking once it has this quality.
    pub cutoff: String,
    /// Every quality the profile allows is better than what's there now.
    pub all_better: bool,
}

/// Words for a profile cutoff rank.
fn cutoff_label(r: Rank) -> String {
    if r.is_lossless() { r.label(Some("FLAC")) } else { r.label(None) }
}

/// Whether a profile aims above `current`.
pub fn qualifies(p: &QualityProfile, current: Rank) -> bool {
    cutoff_rank(p).is_some_and(|c| c > current)
}

/// Pick the profile for an upgrade from what's at `current`: only profiles
/// whose cutoff is above it; ones that allow nothing at or below it first,
/// then the lowest such cutoff (the smallest step that is still better).
pub fn choose_profile(profiles: &[QualityProfile], current: Rank, now_label: &str) -> Result<ProfileChoice, String> {
    let mut best: Option<(bool, Rank, i64, &QualityProfile)> = None;
    for p in profiles {
        let Some(cut) = cutoff_rank(p).filter(|c| *c > current) else { continue };
        let mut ok = Vec::new();
        allowed(&p.items, false, &mut ok);
        let all_better = !ok.is_empty() && ok.iter().all(|r| *r > current);
        let key = (!all_better, cut, p.id, p);
        if best.as_ref().is_none_or(|b| (key.0, key.1, key.2) < (b.0, b.1, b.2)) {
            best = Some(key);
        }
    }
    match best {
        Some((not_all, cut, id, p)) => Ok(ProfileChoice { id, name: p.name.clone(), cutoff: cutoff_label(cut), all_better: !not_all }),
        None => Err(format!(
            "None of Lidarr's quality profiles aims higher than {now_label}, so Lidarr wouldn't look for anything better. Platter never makes or changes profiles: add one in Lidarr (Settings > Profiles) whose cutoff is above {now_label}, then try again."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::super::api::QualityRef;
    use super::*;

    fn q(id: i64, name: &str, allowed: bool) -> ProfileItem {
        ProfileItem { quality: Some(QualityRef { id, name: name.into() }), allowed, ..Default::default() }
    }

    fn group(id: i64, name: &str, items: Vec<ProfileItem>, allowed: bool) -> ProfileItem {
        ProfileItem { id: Some(id), name: Some(name.into()), items, allowed, ..Default::default() }
    }

    fn profile(id: i64, name: &str, cutoff: i64, items: Vec<ProfileItem>) -> QualityProfile {
        QualityProfile { id, name: name.into(), cutoff: Some(cutoff), items, ..Default::default() }
    }

    /// Lidarr's usual profiles: Any (cutoff MP3-320), Lossless (FLAC group),
    /// Standard (MP3s up to 320) and Lossless 24 (cutoff FLAC 24bit).
    fn usual() -> Vec<QualityProfile> {
        let mp3 = || vec![q(1, "MP3-192", true), q(3, "MP3-256", true), q(4, "MP3-320", true)];
        let lossless = |allowed| group(1005, "Lossless", vec![q(6, "FLAC", allowed), q(7, "ALAC", allowed)], allowed);
        vec![
            profile(1, "Any", 4, [mp3(), vec![lossless(true)]].concat()),
            profile(2, "Lossless", 1005, [mp3().into_iter().map(|mut i| { i.allowed = false; i }).collect(), vec![lossless(true)]].concat()),
            profile(3, "Standard", 4, mp3()),
            profile(4, "Lossless 24", 21, vec![lossless(true), q(21, "FLAC 24bit", true)]),
        ]
    }

    #[test]
    fn ranks_order_lossy_below_lossless_by_depth() {
        let mp3_320 = Rank::of_file(Some("mp3"), Some(320), None);
        let aac_256 = Rank::of_file(Some("aac"), Some(256), None);
        let flac = Rank::of_file(Some("flac"), Some(900), Some(16));
        let flac24 = Rank::of_file(Some("FLAC"), None, Some(24));
        let flac8 = Rank::of_file(Some("flac"), None, Some(8));
        assert!(aac_256 < mp3_320 && mp3_320 < flac8 && flac8 < flac && flac < flac24);
        assert_eq!(flac, CD);
        assert_eq!(mp3_320.label(Some("mp3")), "MP3 320");
        assert_eq!(flac.label(Some("flac")), "FLAC");
        assert_eq!(flac24.label(Some("flac")), "FLAC 24-bit");
        assert_eq!(Rank::of_lidarr("MP3-320"), Rank(320));
        assert_eq!(Rank::of_lidarr("FLAC 24bit"), Rank::lossless(Some(24)));
        assert_eq!(Rank::of_lidarr("ALAC"), CD);
        assert!(Rank::of_lidarr("MP3-VBR-V0") > Rank::of_lidarr("MP3-VBR-V2"));
        assert_eq!(Rank::of_lidarr("Unknown"), Rank(0));
    }

    #[test]
    fn cutoffs_resolve_through_groups() {
        let p = usual();
        assert_eq!(cutoff_rank(&p[0]), Some(Rank(320)));
        assert_eq!(cutoff_rank(&p[1]), Some(CD), "a group's cutoff is met by any of its qualities");
        assert_eq!(cutoff_rank(&p[3]), Some(Rank::lossless(Some(24))));
        assert_eq!(cutoff_rank(&QualityProfile { cutoff: Some(99), ..p[0].clone() }), None);
    }

    #[test]
    fn the_cutoff_must_be_above_the_files_now() {
        let p = usual();
        // MP3 320: Lossless allows nothing lossy, so it wins over Lossless 24's higher cutoff and over Any.
        let c = choose_profile(&p, Rank(320), "MP3 320").unwrap();
        assert_eq!((c.id, c.name.as_str(), c.cutoff.as_str(), c.all_better), (2, "Lossless", "FLAC", true));
        // MP3 192: Standard's cutoff (320) is above it too, but it allows 192, so Lossless still comes first.
        assert_eq!(choose_profile(&p, Rank(192), "MP3 192").unwrap().id, 2);
        // FLAC 16-bit: only the 24-bit profile aims higher.
        let c = choose_profile(&p, CD, "FLAC").unwrap();
        assert_eq!((c.id, c.cutoff.as_str()), (4, "FLAC 24-bit"));
        // FLAC 24-bit: nothing qualifies, and it says so plainly.
        let e = choose_profile(&p, Rank::lossless(Some(24)), "FLAC 24-bit").unwrap_err();
        assert!(e.contains("None of Lidarr's quality profiles aims higher than FLAC 24-bit"), "{e}");
        assert!(e.contains("never makes or changes profiles"), "{e}");
        // Only lossy profiles: an MP3 320 album has nowhere to go.
        assert!(choose_profile(&p[2..3], Rank(320), "MP3 320").is_err());
        assert_eq!(choose_profile(&p[2..3], Rank(192), "MP3 192").unwrap().id, 3);
    }
}
