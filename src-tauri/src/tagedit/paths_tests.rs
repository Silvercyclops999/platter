//! The music folder mapping: Windows drives, UNC paths, Mac mounts, and
//! folders that don't match.

use std::collections::HashMap;

use super::paths::{Mapping, detect, join, verify};

fn on_disk(files: &[&str]) -> impl Fn(&str) -> bool {
    let set: Vec<String> = files.iter().map(|s| s.to_string()).collect();
    move |p: &str| set.iter().any(|f| f == p || f.starts_with(&format!("{p}{}", if p.contains('\\') { '\\' } else { '/' })))
}

const LINUX: &str = "/data/media/music/Demo Artist/Demo Album/01 Demo Song.flac";

#[test]
fn a_windows_drive_maps_a_linux_server() {
    let disk = on_disk(&["M:\\Demo Artist\\Demo Album\\01 Demo Song.flac"]);
    let m = detect("M:\\", &[LINUX.to_string()], disk).unwrap();
    assert_eq!(m, Mapping { server: "/data/media/music".into(), local: "M:\\".into() });
    assert_eq!(
        m.to_local("/data/media/music/Other Artist/Other Album/02 Other.mp3").as_deref(),
        Some("M:\\Other Artist\\Other Album\\02 Other.mp3")
    );
    assert_eq!(m.to_local("/data/lidarr/Other Artist/x.flac"), None, "another Plex folder isn't under it");
    assert_eq!(m.to_local("data/media/music/a/b.flac"), None, "relative isn't the same path");
}

#[test]
fn a_unc_path_maps_with_the_folder_above_the_artists() {
    // The share holds a "music" folder: the end found is longer.
    let disk = on_disk(&["\\\\nas\\media\\music\\Demo Artist\\Demo Album\\01 Demo Song.flac"]);
    let m = detect("\\\\nas\\media\\", &[LINUX.to_string()], disk).unwrap();
    assert_eq!(m.server, "/data/media");
    assert_eq!(
        m.to_local(LINUX).as_deref(),
        Some("\\\\nas\\media\\music\\Demo Artist\\Demo Album\\01 Demo Song.flac")
    );
}

#[test]
fn a_mac_mount_maps_a_windows_server() {
    let server = "D:\\Music\\Demo Artist\\Demo Album\\01 Demo Song.flac".to_string();
    let disk = on_disk(&["/Volumes/Music/Demo Artist/Demo Album/01 Demo Song.flac"]);
    let m = detect("/Volumes/Music", &[server], disk).unwrap();
    assert_eq!(m.server, "D:\\Music");
    // Windows paths compare in any case.
    assert_eq!(
        m.to_local("d:\\music\\Demo Artist\\Second\\03 Song.m4a").as_deref(),
        Some("/Volumes/Music/Demo Artist/Second/03 Song.m4a")
    );
}

#[test]
fn a_folder_that_isnt_the_music_is_not_mapped() {
    let disk = on_disk(&["/Volumes/Photos/2019/img.jpg"]);
    assert_eq!(detect("/Volumes/Photos", &[LINUX.to_string()], disk), None);
    // A file name alone matching isn't enough.
    let disk = on_disk(&["/Volumes/Other/01 Demo Song.flac"]);
    assert_eq!(detect("/Volumes/Other", &[LINUX.to_string()], disk), None);
    let r = verify(None, &[(LINUX.into(), Some(10))], |_| None);
    assert!(!r.ok && r.message.contains("None of the songs"), "{}", r.message);
}

#[test]
fn verify_counts_found_files_and_says_whats_wrong() {
    let m = Mapping { server: "/data/media/music".into(), local: "/Volumes/Music".into() };
    let files: Vec<(String, Option<u64>)> = (1..=5).map(|i| (format!("/data/media/music/A/B/0{i}.flac"), Some(100))).collect();
    let mut sizes: HashMap<String, u64> = (1..=5).map(|i| (format!("/Volumes/Music/A/B/0{i}.flac"), 100)).collect();
    let r = verify(Some(&m), &files, |p| sizes.get(p).copied());
    assert!(r.ok);
    assert_eq!(r.message, "Connected, 5 of 5 files found");
    sizes.remove("/Volumes/Music/A/B/05.flac");
    let r = verify(Some(&m), &files, |p| sizes.get(p).copied());
    assert!(!r.ok);
    assert_eq!(r.message, "4 of 5 files found. The others aren't in this folder: it may only hold part of your library.");
    sizes.insert("/Volumes/Music/A/B/05.flac".into(), 99);
    let r = verify(Some(&m), &files, |p| sizes.get(p).copied());
    assert!(r.message.contains("1 has a different size"), "{}", r.message);
    assert!(r.probes[4].found && r.probes[4].size_differs);
}

#[test]
fn join_uses_the_folders_own_separator() {
    assert_eq!(join("M:\\", &["a", "b"]), "M:\\a\\b");
    assert_eq!(join("\\\\nas\\music", &["a"]), "\\\\nas\\music\\a");
    assert_eq!(join("/Volumes/Music/", &["a"]), "/Volumes/Music/a");
    assert_eq!(join("/", &["a"]), "/a");
}
