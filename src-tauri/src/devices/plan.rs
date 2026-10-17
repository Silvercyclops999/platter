//! What a sync does, worked out before anything is written: which songs
//! fit (in the order they were chosen), where each goes, which files on the
//! device stay as they are, which are made, and which of Platter's own files
//! go. Pure: the device is seen only through `on_disk`.

use std::collections::{HashMap, HashSet};

use super::manifest::{Entry, Manifest};
use super::names;
use super::DeviceFormat;
use crate::model::Track;

/// Room kept free on the device beyond what the plan fills.
pub const SPACE_MARGIN: u64 = 32 * 1024 * 1024;

/// A song the device should hold.
#[derive(Debug, Clone)]
pub struct Want {
    pub track: Track,
    /// Index of the selection item it came from.
    pub item: usize,
    pub format: DeviceFormat,
    /// Where it goes when nothing else is there.
    pub natural: String,
    pub source: String,
    /// Estimated size once made.
    pub est: u64,
}

/// A song with its place on the device.
#[derive(Debug, Clone)]
pub struct Placed {
    pub want: Want,
    pub rel: String,
}

#[derive(Debug, Default)]
pub struct Plan {
    /// Already there and unchanged.
    pub keep: Vec<Placed>,
    /// To make, in selection order.
    pub writes: Vec<Placed>,
    /// Platter's files for songs no longer chosen (only when removing is on).
    pub deletes: Vec<String>,
    /// Older copies of songs being made again; they go once the new file is
    /// written (unless it's written over them).
    pub replaced: Vec<String>,
    /// Manifest entries whose file is gone from the device.
    pub stale: Vec<String>,
    /// Songs that didn't fit.
    pub left_out: Vec<Want>,
    /// Bytes the plan fills (kept plus estimated new).
    pub bytes: u64,
}

/// The device's state as far as planning needs it.
pub struct Device<'a> {
    pub manifest: &'a Manifest,
    /// The size of a file at a path below the music folder, None if absent.
    pub on_disk: &'a dyn Fn(&str) -> Option<u64>,
    /// Free space now.
    pub free: Option<u64>,
}

fn unchanged(e: &Entry, w: &Want, size: Option<u64>) -> bool {
    e.track_id == w.track.id && e.source == w.source && e.format == w.format && size == Some(e.size)
}

pub fn plan(wanted: Vec<Want>, dev: &Device<'_>, budget: Option<u64>, delete_removed: bool) -> Plan {
    let m = dev.manifest;
    // One copy of each song, from the first item that has it.
    let mut seen = HashSet::new();
    let wanted: Vec<Want> = wanted.into_iter().filter(|w| seen.insert(w.track.id.clone())).collect();
    let wanted_ids: HashSet<&str> = wanted.iter().map(|w| w.track.id.as_str()).collect();

    // Files on the device now, from the manifest.
    let sizes: HashMap<&str, Option<u64>> = m.files.keys().map(|p| (p.as_str(), (dev.on_disk)(p))).collect();
    let mut by_track: HashMap<&str, Vec<&str>> = HashMap::new();
    for (p, e) in &m.files {
        by_track.entry(e.track_id.as_str()).or_default().push(p.as_str());
    }
    let kept_path = |w: &Want| -> Option<String> {
        by_track
            .get(w.track.id.as_str())?
            .iter()
            .find(|p| unchanged(&m.files[**p], w, sizes[**p]))
            .map(|p| p.to_string())
    };

    // Space: what's free, plus what Platter's own files hold that the plan
    // may reuse or remove.
    let reclaim: u64 = m
        .files
        .iter()
        .filter(|(p, e)| sizes[p.as_str()].is_some() && (delete_removed || wanted_ids.contains(e.track_id.as_str())))
        .map(|(_, e)| e.size)
        .sum();
    let space = dev.free.map(|f| (f + reclaim).saturating_sub(SPACE_MARGIN));
    let cap = match (budget, space) {
        (Some(b), Some(s)) => Some(b.min(s)),
        (b, s) => b.or(s),
    };

    // Fill in order; a song that doesn't fit is left out and smaller ones
    // after it still get their turn.
    let mut out = Plan::default();
    let mut included = Vec::new();
    for w in wanted {
        let size = kept_path(&w).map(|p| m.files[&p].size).unwrap_or(w.est);
        if cap.is_some_and(|c| out.bytes + size > c) {
            out.left_out.push(w);
            continue;
        }
        out.bytes += size;
        included.push(w);
    }
    let included_ids: HashSet<String> = included.iter().map(|w| w.track.id.clone()).collect();

    // Kept files hold their paths first.
    let mut taken: HashSet<String> = HashSet::new();
    let mut keep_paths: HashSet<String> = HashSet::new();
    let mut to_write = Vec::new();
    for w in included {
        match kept_path(&w) {
            Some(p) => {
                taken.insert(names::key(&p));
                keep_paths.insert(p.clone());
                out.keep.push(Placed { want: w, rel: p });
            }
            None => to_write.push(w),
        }
    }

    // What happens to every other file Platter wrote.
    for (p, e) in &m.files {
        if keep_paths.contains(p) {
            continue;
        }
        if sizes[p.as_str()].is_none() {
            out.stale.push(p.clone());
        } else if included_ids.contains(&e.track_id) {
            out.replaced.push(p.clone());
        } else if delete_removed {
            out.deletes.push(p.clone());
        } else {
            // Stays on the device as it is.
            taken.insert(names::key(p));
        }
    }
    for p in m.playlists.keys() {
        taken.insert(names::key(p));
    }
    let ours: HashSet<String> = m.files.keys().map(|p| names::key(p)).collect();
    // An old copy being replaced is removed once its song's new file is
    // written, so only that song may be written over it.
    let replaced_by: HashMap<String, &str> = out.replaced.iter().map(|p| (names::key(p), m.files[p].track_id.as_str())).collect();

    // New files: the natural path, or the next free ` (n)` one. A file
    // Platter didn't write is never written over.
    for w in to_write {
        let mut n = 1;
        let rel = loop {
            let rel = if n == 1 { w.natural.clone() } else { names::with_suffix(&w.natural, n) };
            let k = names::key(&rel);
            let foreign = !ours.contains(&k) && (dev.on_disk)(&rel).is_some();
            let others_old = replaced_by.get(&k).is_some_and(|t| *t != w.track.id);
            if !taken.contains(&k) && !foreign && !others_old {
                break rel;
            }
            n += 1;
        };
        taken.insert(names::key(&rel));
        out.writes.push(Placed { want: w, rel });
    }
    // An old copy written over by its new file needn't be deleted.
    let written: HashSet<String> = out.writes.iter().map(|p| names::key(&p.rel)).collect();
    out.replaced.retain(|p| !written.contains(&names::key(p)));
    out.deletes.retain(|p| !written.contains(&names::key(p)));
    out
}

/// A rough size for a song once made, for the budget.
pub fn estimate(t: &Track, format: DeviceFormat, copies: bool) -> u64 {
    let secs = (t.duration_ms / 1000).max(1);
    let original = match t.bitrate {
        Some(k) if k > 0 => k as u64 * 1000 / 8 * secs,
        _ => crate::downloads::estimate_bytes(t, crate::downloads::DownloadQuality::Original),
    };
    let kbps: u64 = match format {
        DeviceFormat::Original => return original + 1024,
        _ if copies => return original + 64 * 1024,
        DeviceFormat::Flac => 900,
        DeviceFormat::Mp3Cbr320 => 320,
        DeviceFormat::Mp3V0 => 260,
        DeviceFormat::Opus => 165,
    };
    // Plus the cover and tags.
    kbps * 1000 / 8 * secs + 64 * 1024
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::fake;

    fn want(id: &str, natural: &str, source: &str) -> Want {
        let ar = fake::artist("ar", "Artist");
        let al = fake::album("al", "Album", &ar, 2000, None, &[]);
        Want {
            track: fake::track(id, id, &al, 1, 1),
            item: 0,
            format: DeviceFormat::Original,
            natural: natural.into(),
            source: source.into(),
            est: 10,
        }
    }

    fn entry(track: &str, source: &str) -> Entry {
        Entry { track_id: track.into(), source: source.into(), format: DeviceFormat::Original, size: 10 }
    }

    #[test]
    fn another_songs_old_copy_is_never_a_new_files_place() {
        // Song a is made again (its source changed); its old copy sits where
        // song b would naturally go, apart from case.
        let mut m = Manifest::new("d");
        m.files.insert("X/01 Same.flac".into(), entry("a", "old"));
        let on_disk = |p: &str| (p == "X/01 Same.flac").then_some(10);
        let dev = Device { manifest: &m, on_disk: &on_disk, free: None };
        let p = plan(vec![want("a", "Y/01 A.flac", "new"), want("b", "x/01 same.flac", "s")], &dev, None, false);
        let b = p.writes.iter().find(|w| w.want.track.id == "b").unwrap();
        assert_ne!(names::key(&b.rel), names::key("X/01 Same.flac"), "b must not land on a's old copy");
        assert_eq!(p.replaced, vec!["X/01 Same.flac".to_string()]);
    }

    #[test]
    fn a_song_may_be_written_over_its_own_old_copy_in_new_case() {
        let mut m = Manifest::new("d");
        m.files.insert("X/01 Song.flac".into(), entry("a", "old"));
        let on_disk = |p: &str| (p == "X/01 Song.flac").then_some(10);
        let dev = Device { manifest: &m, on_disk: &on_disk, free: None };
        let p = plan(vec![want("a", "x/01 song.flac", "new")], &dev, None, false);
        assert_eq!(p.writes[0].rel, "x/01 song.flac");
        assert!(p.replaced.is_empty(), "written over, nothing to remove");
    }
}
