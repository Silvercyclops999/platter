//! File and folder names that every player's file system takes: FAT32 and
//! exFAT refuse `"*/:<>?\|`, control characters, names ending in a dot or a
//! space and the old device names (CON, NUL...), and they don't tell upper
//! from lower case. Paths stay short enough for Windows' 260 character limit
//! with the drive and music folder in front.

use unicode_normalization::UnicodeNormalization;

/// Longest folder or file name, in characters.
pub const MAX_COMPONENT: usize = 80;
/// Longest path below the music folder, in characters.
pub const MAX_PATH: usize = 180;

const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// One safe folder or file name (no extension) from any text. `fallback`
/// when nothing usable is left.
pub fn component(s: &str, fallback: &str, max: usize) -> String {
    let mut out: String = s
        .nfc()
        .map(|c| match c {
            '"' => '\'',
            '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|' => '_',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    // Runs of spaces become one.
    let mut squeezed = String::with_capacity(out.len());
    for c in out.chars() {
        if c == ' ' && squeezed.ends_with(' ') {
            continue;
        }
        squeezed.push(c);
    }
    out = squeezed;
    out = truncate(out.trim(), max);
    let trimmed = out.trim_end_matches(['.', ' ']).trim_start();
    let mut out = trimmed.to_string();
    if out.is_empty() {
        out = fallback.to_string();
    }
    let stem = out.split('.').next().unwrap_or_default().trim_end().to_ascii_uppercase();
    if RESERVED.contains(&stem.as_str()) {
        out.insert(0, '_');
    }
    out
}

/// Most file systems cap a name at 255 bytes of UTF-8. A name stays below
/// that with room for its extension and a ` (n)` suffix.
pub const MAX_COMPONENT_BYTES: usize = 240;

/// At most `max` characters and [`MAX_COMPONENT_BYTES`] bytes, cut on a
/// character.
fn truncate(s: &str, max: usize) -> String {
    let mut out = String::new();
    for c in s.chars().take(max) {
        if out.len() + c.len_utf8() > MAX_COMPONENT_BYTES {
            break;
        }
        out.push(c);
    }
    out
}

/// Where a song goes below the music folder: `Artist/Album/NN Title.ext`,
/// with the disc in front (`2-03 Title`) on albums of more than one disc.
pub fn track_path(artist: &str, album: &str, disc: u32, multi_disc: bool, number: u32, title: &str, ext: &str) -> String {
    let num = match (number, multi_disc) {
        (0, false) => String::new(),
        (0, true) => format!("{}-", disc.max(1)),
        (n, false) => format!("{n:02} "),
        (n, true) => format!("{}-{n:02} ", disc.max(1)),
    };
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let mut lens = [MAX_COMPONENT, MAX_COMPONENT, MAX_COMPONENT];
    loop {
        let a = component(artist, "Unknown Artist", lens[0]);
        let b = component(album, "Unknown Album", lens[1]);
        let t = component(&format!("{num}{title}"), "Untitled", lens[2]);
        let path = format!("{a}/{b}/{t}.{ext}");
        let n = path.chars().count();
        if n <= MAX_PATH || lens.iter().all(|l| *l <= 20) {
            return path;
        }
        // Shorten the longest part first.
        let parts = [a.chars().count(), b.chars().count(), t.chars().count()];
        let i = (0..3).max_by_key(|i| parts[*i]).unwrap_or(2);
        lens[i] = parts[i].saturating_sub(n - MAX_PATH).max(20);
    }
}

/// A playlist file's name at the top of the music folder.
pub fn playlist_path(title: &str) -> String {
    format!("{}.m3u8", component(title, "Playlist", MAX_COMPONENT))
}

/// The path with ` (n)` before its extension, for a name already taken.
pub fn with_suffix(rel: &str, n: u32) -> String {
    let (dir, file) = match rel.rfind('/') {
        Some(i) => (&rel[..=i], &rel[i + 1..]),
        None => ("", rel),
    };
    match file.rfind('.') {
        Some(i) if i > 0 => format!("{dir}{} ({n}){}", &file[..i], &file[i..]),
        _ => format!("{dir}{file} ({n})"),
    }
}

/// How the file system compares names: case and composed form ignored.
pub fn key(rel: &str) -> String {
    rel.nfc().collect::<String>().to_lowercase()
}

/// The file extension of a song's original file, from its path on the
/// server, else its codec or container.
/// Extensions a copied song may get on the device; anything else falls back
/// to the codec's, so a server can't put a program or shortcut on the drive.
const AUDIO_EXTS: &[&str] =
    &["flac", "mp3", "m4a", "m4b", "aac", "alac", "ogg", "oga", "opus", "wav", "aif", "aiff", "wma", "ape", "wv", "dsf", "dff", "mka", "mpc"];

pub fn original_ext(part_key: &str, codec: Option<&str>, container: Option<&str>) -> String {
    let name = part_key.rsplit('/').next().unwrap_or_default();
    if let Some((_, ext)) = name.rsplit_once('.') {
        let ext = ext.split(['?', '&']).next().unwrap_or_default().to_ascii_lowercase();
        if AUDIO_EXTS.contains(&ext.as_str()) {
            return ext;
        }
    }
    crate::audio::decode::extension_hint(codec, container).unwrap_or_else(|| "audio".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_characters_are_replaced() {
        assert_eq!(component("AC/DC: Live? <1991>", "x", 80), "AC_DC_ Live_ _1991_");
        assert_eq!(component("Say \"hi\"", "x", 80), "Say 'hi'");
        assert_eq!(component("tab\there", "x", 80), "tab here");
        assert_eq!(component("a|b*c\\d", "x", 80), "a_b_c_d");
    }

    #[test]
    fn trailing_dots_and_spaces_go() {
        assert_eq!(component("Vol. 2...", "x", 80), "Vol. 2");
        assert_eq!(component("  Name  ", "x", 80), "Name");
        assert_eq!(component("...", "Untitled", 80), "Untitled");
        assert_eq!(component("", "Unknown Artist", 80), "Unknown Artist");
    }

    #[test]
    fn device_names_get_an_underscore() {
        assert_eq!(component("CON", "x", 80), "_CON");
        assert_eq!(component("nul", "x", 80), "_nul");
        assert_eq!(component("Com1.txt", "x", 80), "_Com1.txt");
        assert_eq!(component("Console", "x", 80), "Console");
    }

    #[test]
    fn long_names_are_cut_on_characters() {
        let long = "é".repeat(200);
        let c = component(&long, "x", 80);
        assert_eq!(c.chars().count(), 80);
    }

    #[test]
    fn long_names_stay_under_255_bytes() {
        // Four bytes a character: 80 of them would be 320 bytes.
        let wide = "\u{1F3B5}".repeat(200);
        let c = component(&wide, "x", MAX_COMPONENT);
        assert!(c.len() <= MAX_COMPONENT_BYTES, "{}", c.len());
        assert_eq!(c.chars().count(), MAX_COMPONENT_BYTES / 4);
        let cjk = "\u{97F3}".repeat(200);
        let p = track_path(&cjk, &cjk, 1, false, 12, &cjk, "flac");
        for part in p.split('/') {
            assert!(part.len() <= 255, "{} bytes", part.len());
        }
        assert!(with_suffix(&p, 9999).split('/').all(|part| part.len() <= 255));
    }

    #[test]
    fn layout_is_artist_album_number_title() {
        assert_eq!(track_path("Artist", "Album", 1, false, 3, "Song", "FLAC"), "Artist/Album/03 Song.flac");
        assert_eq!(track_path("Artist", "Album", 2, true, 7, "Song", "mp3"), "Artist/Album/2-07 Song.mp3");
        assert_eq!(track_path("", "", 1, false, 0, "", "opus"), "Unknown Artist/Unknown Album/Untitled.opus");
    }

    #[test]
    fn paths_stay_under_the_limit() {
        let long = "x".repeat(300);
        let p = track_path(&long, &long, 1, false, 1, &long, "flac");
        assert!(p.chars().count() <= MAX_PATH, "{}", p.chars().count());
        assert!(p.ends_with(".flac"));
        assert_eq!(p.split('/').count(), 3);
    }

    #[test]
    fn suffixes_go_before_the_extension() {
        assert_eq!(with_suffix("A/B/01 Song.mp3", 2), "A/B/01 Song (2).mp3");
        assert_eq!(with_suffix("List.m3u8", 3), "List (3).m3u8");
        assert_eq!(with_suffix("noext", 2), "noext (2)");
    }

    #[test]
    fn keys_ignore_case_and_composition() {
        assert_eq!(key("Beyonce\u{301}/X.mp3"), key("BEYONCÉ/x.MP3"));
    }

    #[test]
    fn original_extension_from_the_part() {
        assert_eq!(original_ext("/library/parts/1/2/file.FLAC", None, None), "flac");
        assert_eq!(original_ext("/library/parts/1/2/file", Some("mp3"), None), "mp3");
        assert_eq!(original_ext("/rest/stream?id=3", Some("aac"), Some("mp4")), "m4a");
        assert_eq!(original_ext("x", None, None), "audio");
        assert_eq!(original_ext("/library/parts/1/evil.lnk", Some("mp3"), None), "mp3");
    }
}
