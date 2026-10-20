//! The Music app's library file (File > Library > Export Library): a
//! property list in XML holding every track and every playlist.
//!
//! - Tracks: `Name`, `Artist` (else `Album Artist`), `Album`, `Total Time`
//!   (ms). Loved or favourited tracks (`Loved`, `Favorited`) become the
//!   liked songs. Podcasts, videos, audiobooks and voice memos are left out.
//! - Playlists: the user's own lists with their `Playlist Items`. The whole
//!   library, folders, hidden lists and the app's built-in lists (they carry
//!   `Master` or `Distinguished Kind`) are left out.
//!
//! Only names and lengths are read. File locations, play dates, ratings and
//! the like are never copied.

use std::collections::HashMap;

use quick_xml::Reader;
use quick_xml::events::Event;

use super::Parser;
use crate::error::{AppError, AppResult};
use crate::import::files::{InputFile, decode_text};
use crate::import::{Parsed, PlaylistIn, SongIn};

pub struct AppleLibrary;

/// A property list value.
#[derive(Debug, Clone, PartialEq)]
enum Pv {
    Dict(Vec<(String, Pv)>),
    Array(Vec<Pv>),
    Str(String),
    Int(i64),
    Bool(bool),
    Other,
}

impl Pv {
    fn get(&self, key: &str) -> Option<&Pv> {
        match self {
            Pv::Dict(d) => d.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Pv::Str(s)) => Some(s.trim()).filter(|s| !s.is_empty()),
            _ => None,
        }
    }
    fn int(&self, key: &str) -> Option<i64> {
        match self.get(key) {
            Some(Pv::Int(n)) => Some(*n),
            Some(Pv::Str(s)) => s.trim().parse().ok(),
            _ => None,
        }
    }
    fn yes(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Pv::Bool(true)))
    }
}

enum Frame {
    Dict(Vec<(String, Pv)>, Option<String>),
    Array(Vec<Pv>),
}

fn push(stack: &mut [Frame], root: &mut Option<Pv>, v: Pv) {
    match stack.last_mut() {
        Some(Frame::Dict(d, key)) => d.push((key.take().unwrap_or_default(), v)),
        Some(Frame::Array(a)) => a.push(v),
        None => {
            root.get_or_insert(v);
        }
    }
}

/// The text of a predefined entity.
fn entity(name: &str) -> Option<char> {
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ => return None,
    })
}

/// Reads a property list in XML. None when it isn't one.
fn parse_plist(text: &str) -> Option<Pv> {
    let mut r = Reader::from_str(text);
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<Pv> = None;
    // The element whose text is being read, and the text so far.
    let mut leaf: Option<(String, String)> = None;
    let mut seen_plist = false;
    loop {
        match r.read_event().ok()? {
            Event::Start(e) => {
                let name = e.name().0.to_string();
                match name.as_str() {
                    "plist" => seen_plist = true,
                    "dict" => stack.push(Frame::Dict(Vec::new(), None)),
                    "array" => stack.push(Frame::Array(Vec::new())),
                    _ => leaf = Some((name, String::new())),
                }
            }
            Event::Empty(e) => {
                let v = match e.name().0 {
                    "true" => Pv::Bool(true),
                    "false" => Pv::Bool(false),
                    "dict" => Pv::Dict(Vec::new()),
                    "array" => Pv::Array(Vec::new()),
                    "string" => Pv::Str(String::new()),
                    "key" => {
                        if let Some(Frame::Dict(_, k)) = stack.last_mut() {
                            *k = Some(String::new());
                        }
                        continue;
                    }
                    _ => Pv::Other,
                };
                push(&mut stack, &mut root, v);
            }
            Event::Text(t) => {
                if let Some((_, s)) = leaf.as_mut() {
                    s.push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Event::CData(t) => {
                if let Some((_, s)) = leaf.as_mut() {
                    s.push_str(&t.xml_content(quick_xml::XmlVersion::Implicit1_0));
                }
            }
            Event::GeneralRef(g) => {
                if let Some((_, s)) = leaf.as_mut() {
                    let c = match g.resolve_char_ref() {
                        Ok(Some(c)) => Some(c),
                        _ => entity(&g.xml_content(quick_xml::XmlVersion::Implicit1_0)),
                    };
                    if let Some(c) = c {
                        s.push(c);
                    }
                }
            }
            Event::End(e) => match e.name().0 {
                "plist" => {}
                "dict" | "array" => {
                    let v = match stack.pop()? {
                        Frame::Dict(d, _) => Pv::Dict(d),
                        Frame::Array(a) => Pv::Array(a),
                    };
                    push(&mut stack, &mut root, v);
                }
                _ => {
                    let (name, text) = leaf.take()?;
                    if name == "key" {
                        if let Some(Frame::Dict(_, k)) = stack.last_mut() {
                            *k = Some(text);
                        }
                        continue;
                    }
                    let v = match name.as_str() {
                        "string" => Pv::Str(text),
                        "integer" => text.trim().parse().map(Pv::Int).unwrap_or(Pv::Other),
                        _ => Pv::Other,
                    };
                    push(&mut stack, &mut root, v);
                }
            },
            Event::Eof => break,
            _ => {}
        }
    }
    root.filter(|_| seen_plist)
}

/// Whether the bytes look like the Music app's library file (cheap: the
/// start of the file only).
pub fn looks_like(f: &InputFile) -> bool {
    if f.ext() != "xml" {
        return false;
    }
    let head = String::from_utf8_lossy(&f.bytes[..f.bytes.len().min(4096)]).into_owned();
    head.contains("<plist") && head.contains("<key>Tracks</key>")
}

/// Tracks that aren't music.
fn not_music(t: &Pv) -> bool {
    let kind = t.str("Kind").unwrap_or_default().to_ascii_lowercase();
    t.yes("Podcast")
        || t.yes("Movie")
        || t.yes("TV Show")
        || t.yes("Has Video")
        || t.yes("Music Video")
        || t.yes("Audiobook")
        || t.yes("Voice Memo")
        || kind.contains("video")
        || kind.contains("audiobook")
        || kind.contains("podcast")
}

/// Playlists that are the app's own, not the user's.
fn built_in(p: &Pv) -> bool {
    p.yes("Master") || p.get("Distinguished Kind").is_some() || p.yes("Folder") || matches!(p.get("Visible"), Some(Pv::Bool(false)))
}

fn read_library(root: &Pv, out: &mut Parsed) -> bool {
    let Some(Pv::Dict(tracks)) = root.get("Tracks") else { return false };
    let mut songs: HashMap<i64, SongIn> = HashMap::new();
    let mut liked = Vec::new();
    for (key, t) in tracks {
        let id = t.int("Track ID").or_else(|| key.parse().ok());
        if not_music(t) {
            out.skipped.podcasts += 1;
            continue;
        }
        let (Some(id), Some(title), Some(artist)) = (id, t.str("Name"), t.str("Artist").or_else(|| t.str("Album Artist"))) else {
            out.skipped.incomplete_rows += 1;
            continue;
        };
        let song = SongIn {
            artist: artist.to_string(),
            title: title.to_string(),
            album: t.str("Album").map(str::to_string),
            duration_ms: t.int("Total Time").filter(|n| *n > 0).map(|n| n as u64),
            isrc: None,
            service_id: None,
        };
        if t.yes("Loved") || t.yes("Favorited") {
            liked.push(song.clone());
        }
        songs.insert(id, song);
    }
    if let Some(Pv::Array(lists)) = root.get("Playlists") {
        for p in lists {
            if built_in(p) {
                continue;
            }
            let Some(name) = p.str("Name") else { continue };
            let items: Vec<SongIn> = match p.get("Playlist Items") {
                Some(Pv::Array(items)) => items.iter().filter_map(|i| i.int("Track ID")).filter_map(|id| songs.get(&id).cloned()).collect(),
                _ => Vec::new(),
            };
            if !items.is_empty() {
                out.playlists.push(PlaylistIn { name: name.to_string(), songs: items });
            }
        }
    }
    if !liked.is_empty() {
        out.liked.get_or_insert_with(Vec::new).extend(liked);
    }
    true
}

impl Parser for AppleLibrary {
    fn parse(&self, files: &[InputFile]) -> AppResult<Parsed> {
        let mut out = Parsed::default();
        let mut usable = 0;
        for f in files {
            if f.ext() != "xml" {
                out.skipped.unknown_files += 1;
                continue;
            }
            let text = decode_text(&f.bytes);
            match parse_plist(&text) {
                Some(root) if read_library(&root, &mut out) => usable += 1,
                _ => out.skipped.unknown_files += 1,
            }
        }
        if usable == 0 {
            return Err(AppError::Invalid(
                "That isn't a Music library file. In the Music app choose File, Library, Export Library, then pick the file it saves.".into(),
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIB: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Major Version</key><integer>1</integer>
	<key>Tracks</key>
	<dict>
		<key>101</key>
		<dict>
			<key>Track ID</key><integer>101</integer>
			<key>Name</key><string>Night &amp; Day</string>
			<key>Artist</key><string>Northlight</string>
			<key>Album</key><string>Tides</string>
			<key>Total Time</key><integer>201000</integer>
			<key>Loved</key><true/>
			<key>Location</key><string>file:///Users/someone/Music/x.m4a</string>
		</dict>
		<key>102</key>
		<dict>
			<key>Track ID</key><integer>102</integer>
			<key>Name</key><string>Harbour</string>
			<key>Album Artist</key><string>Northlight</string>
			<key>Album</key><string>Tides</string>
			<key>Favorited</key><true/>
		</dict>
		<key>103</key>
		<dict>
			<key>Track ID</key><integer>103</integer>
			<key>Name</key><string>Episode 4</string>
			<key>Artist</key><string>A Show</string>
			<key>Podcast</key><true/>
		</dict>
		<key>104</key>
		<dict>
			<key>Track ID</key><integer>104</integer>
			<key>Name</key><string>Glass</string>
			<key>Artist</key><string>Pale Rooms</string>
		</dict>
	</dict>
	<key>Playlists</key>
	<array>
		<dict>
			<key>Name</key><string>Library</string>
			<key>Master</key><true/>
			<key>Playlist Items</key><array><dict><key>Track ID</key><integer>101</integer></dict></array>
		</dict>
		<dict>
			<key>Name</key><string>Music</string>
			<key>Distinguished Kind</key><integer>4</integer>
			<key>Playlist Items</key><array><dict><key>Track ID</key><integer>101</integer></dict></array>
		</dict>
		<dict>
			<key>Name</key><string>Late drives</string>
			<key>Playlist Items</key>
			<array>
				<dict><key>Track ID</key><integer>104</integer></dict>
				<dict><key>Track ID</key><integer>101</integer></dict>
				<dict><key>Track ID</key><integer>999</integer></dict>
			</array>
		</dict>
		<dict>
			<key>Name</key><string>Empty one</string>
		</dict>
	</array>
</dict>
</plist>
"#;

    #[test]
    fn reads_tracks_loved_songs_and_the_users_playlists() {
        let f = InputFile::new("Library.xml", LIB.as_bytes());
        assert!(looks_like(&f));
        let p = AppleLibrary.parse(&[f]).unwrap();
        let liked = p.liked.unwrap();
        assert_eq!(liked.len(), 2);
        assert!(liked.iter().any(|s| s.title == "Night & Day" && s.duration_ms == Some(201_000)));
        assert!(liked.iter().any(|s| s.title == "Harbour" && s.artist == "Northlight"));
        assert_eq!(p.playlists.len(), 1, "built-in lists and empty lists are left out");
        assert_eq!(p.playlists[0].name, "Late drives");
        let titles: Vec<&str> = p.playlists[0].songs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["Glass", "Night & Day"]);
        assert_eq!(p.skipped.podcasts, 1);
    }

    #[test]
    fn no_loved_songs_keeps_earlier_likes() {
        let lib = LIB.replace("<key>Loved</key><true/>", "").replace("<key>Favorited</key><true/>", "");
        let p = AppleLibrary.parse(&[InputFile::new("Library.xml", lib.into_bytes())]).unwrap();
        assert!(p.liked.is_none());
    }

    #[test]
    fn another_xml_file_is_refused_in_plain_words() {
        let e = AppleLibrary.parse(&[InputFile::new("feed.xml", "<rss><channel/></rss>")]).unwrap_err();
        assert!(e.to_string().contains("Export Library"), "{e}");
        assert!(!looks_like(&InputFile::new("feed.xml", "<rss/>")));
    }
}
