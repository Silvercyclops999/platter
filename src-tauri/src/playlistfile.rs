//! Save a playlist as a file (the playlist page's "Save as a file"): a
//! playlist file (.m3u8) with each song's path on the server when the
//! library knows it, or a plain text list of "Artist - Title" lines. The
//! file goes where the user picks in a save dialog; nothing is uploaded.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::Connection;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::db::DbHandle;
use crate::error::{AppError, AppResult};
use crate::model::Track;

/// Songs one file holds at most.
pub const MAX_SONGS: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Extended M3U in UTF-8.
    M3u8,
    /// "Artist - Title" lines.
    Text,
}

impl Format {
    /// The format a picked file name asks for (a .txt is a text list).
    pub fn of(path: &Path) -> Format {
        match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("txt") => Format::Text,
            _ => Format::M3u8,
        }
    }
}

/// One line's worth of text: no line breaks.
fn line(s: &str) -> String {
    s.chars().map(|c| if c == '\r' || c == '\n' { ' ' } else { c }).collect::<String>().trim().to_string()
}

/// "Artist - Title", with the song's own artist when it has one.
fn artist_title(t: &Track) -> String {
    let artist = t.track_artist.as_deref().filter(|a| !a.trim().is_empty()).unwrap_or(&t.artist_title);
    format!("{} - {}", line(artist), line(&t.title))
}

/// A path that means the same wherever the file is saved: a path relative
/// to a library folder would point next to the playlist file instead.
fn is_full_path(p: &str) -> bool {
    let b = p.as_bytes();
    p.starts_with('/') || p.starts_with("\\\\") || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/'))
}

/// The file's text. `paths` maps song ids to their path on the server.
pub fn render(title: &str, tracks: &[Track], paths: &HashMap<String, String>, format: Format) -> String {
    let mut out = String::new();
    match format {
        Format::M3u8 => {
            out.push_str("#EXTM3U\n");
            out.push_str(&format!("#PLAYLIST:{}\n", line(title)));
            for t in tracks {
                let secs = (t.duration_ms / 1000) as i64;
                out.push_str(&format!("#EXTINF:{},{}\n", if secs > 0 { secs } else { -1 }, artist_title(t)));
                // Without a full path, the artist and title stand in, so a person or a matching tool can still find it.
                let entry = match paths.get(&t.id).map(|p| line(p)).filter(|p| is_full_path(p)) {
                    Some(p) => p,
                    None => artist_title(t),
                };
                // A line starting with # would be read as a comment and the song lost.
                if entry.starts_with('#') {
                    out.push_str("./");
                }
                out.push_str(&entry);
                out.push('\n');
            }
        }
        Format::Text => {
            for t in tracks {
                out.push_str(&artist_title(t));
                out.push('\n');
            }
        }
    }
    out
}

/// The songs, in the given order, and the paths the library knows for them.
pub fn load(c: &Connection, ids: &[String]) -> AppResult<(Vec<Track>, HashMap<String, String>)> {
    let tracks = crate::db::queries::tracks_by_ids(c, ids)?;
    let mut paths = HashMap::new();
    let mut st = c.prepare_cached("SELECT file FROM tracks WHERE id = ?1 AND file IS NOT NULL AND file != ''")?;
    for t in &tracks {
        if let Some(p) = rusqlite::OptionalExtension::optional(st.query_row([&t.id], |r| r.get::<_, String>(0)))? {
            paths.insert(t.id.clone(), p);
        }
    }
    Ok((tracks, paths))
}

/// A file name from the playlist's title, safe on every system.
pub fn file_name(title: &str) -> String {
    let mut s: String = title
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() { '_' } else { c })
        .collect();
    s = s.trim().trim_end_matches('.').trim().to_owned();
    if s.chars().count() > 120 {
        s = s.chars().take(120).collect();
    }
    if s.is_empty() {
        s = "Playlist".into();
    }
    format!("{s}.m3u8")
}

/// Write the file, saying why when it can't be written. It is written next
/// to the target first and then moved over it, so a failed save never
/// leaves a file it replaces cut short.
pub fn write(path: &Path, body: &str) -> AppResult<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.saving"));
    let done = std::fs::write(&tmp, body).and_then(|()| std::fs::rename(&tmp, path));
    done.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        AppError::Io(format!("Couldn't save the playlist file: {}.", why(&e)))
    })
}

fn why(e: &std::io::Error) -> String {
    if cfg!(windows) && e.raw_os_error() == Some(32) {
        return "another app has it open".into();
    }
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => "it can't be written there".into(),
        std::io::ErrorKind::NotFound => "that folder isn't there any more".into(),
        std::io::ErrorKind::StorageFull => "the disk is full".into(),
        _ => e.to_string().chars().take(160).collect(),
    }
}

/// `playlist_save_file`: save these songs as a playlist file wherever the
/// user picks. Returns the saved file's name, or None when they cancelled.
#[tauri::command]
pub async fn playlist_save_file(app: AppHandle, db: State<'_, DbHandle>, title: String, track_ids: Vec<String>) -> AppResult<Option<String>> {
    if track_ids.is_empty() {
        return Err(AppError::Invalid("This playlist has no songs to save.".into()));
    }
    if track_ids.len() > MAX_SONGS {
        return Err(AppError::Invalid(format!("A playlist file can hold up to {MAX_SONGS} songs.")));
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Save playlist as a file")
        .set_file_name(file_name(&title))
        .add_filter("Playlist file", &["m3u8"])
        .add_filter("Text list", &["txt"])
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| AppError::Io(format!("save path: {e}")))?;
    let (tracks, paths) = db.read_async(move |c| load(c, &track_ids)).await?;
    if tracks.is_empty() {
        return Err(AppError::Invalid("None of these songs are in the library any more.".into()));
    }
    let body = render(&title, &tracks, &paths, Format::of(&path));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    tokio::task::spawn_blocking(move || write(&path, &body))
        .await
        .map_err(|e| AppError::Internal(format!("save playlist file: {e}")))??;
    Ok(Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, title: &str, artist: &str, by: Option<&str>, ms: u64) -> Track {
        let mut t = crate::platform::media::tests::track(id);
        t.title = title.into();
        t.artist_title = artist.into();
        t.track_artist = by.map(Into::into);
        t.duration_ms = ms;
        t
    }

    fn songs() -> (Vec<Track>, HashMap<String, String>) {
        let tracks = vec![
            track("1", "Blue Hour", "Copperline Choir", None, 241_000),
            track("2", "Ferris\nWheel", "Various Artists", Some("Kesto Marrow"), 0),
        ];
        let paths = HashMap::from([("1".to_string(), "/music/Copperline Choir/Quiet Lanterns/03 Blue Hour.flac".to_string())]);
        (tracks, paths)
    }

    #[test]
    fn a_playlist_file_has_paths_and_falls_back_to_artist_and_title() {
        let (tracks, paths) = songs();
        assert_eq!(
            render("Road trip", &tracks, &paths, Format::M3u8),
            "#EXTM3U\n#PLAYLIST:Road trip\n\
             #EXTINF:241,Copperline Choir - Blue Hour\n/music/Copperline Choir/Quiet Lanterns/03 Blue Hour.flac\n\
             #EXTINF:-1,Kesto Marrow - Ferris Wheel\nKesto Marrow - Ferris Wheel\n"
        );
    }

    #[test]
    fn lines_never_start_with_a_hash_and_partial_paths_are_left_out() {
        let tracks = vec![track("1", "#9", "#1 Dads", None, 1000), track("2", "Two", "Band", None, 1000), track("3", "Three", "Band", None, 1000)];
        let paths = HashMap::from([
            ("2".to_string(), "Band/Album/02 Two.flac".to_string()),
            ("3".to_string(), "D:\\Music\\03 Three.flac".to_string()),
        ]);
        let out = render("x", &tracks, &paths, Format::M3u8);
        let entries: Vec<&str> = out.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(entries, ["./#1 Dads - #9", "Band - Two", "D:\\Music\\03 Three.flac"]);
        assert!(is_full_path("\\\\nas\\music\\a.flac"));
        assert!(!is_full_path("C:music"));
    }

    #[test]
    fn a_text_list_is_artist_and_title() {
        let (tracks, paths) = songs();
        assert_eq!(render("x", &tracks, &paths, Format::Text), "Copperline Choir - Blue Hour\nKesto Marrow - Ferris Wheel\n");
        assert_eq!(Format::of(Path::new("/tmp/list.TXT")), Format::Text);
        assert_eq!(Format::of(Path::new("/tmp/list.m3u8")), Format::M3u8);
        assert_eq!(Format::of(Path::new("/tmp/list")), Format::M3u8);
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("AC/DC: best?"), "AC_DC_ best_.m3u8");
        assert_eq!(file_name("  ...  "), "Playlist.m3u8");
    }

    #[test]
    fn songs_come_in_the_order_asked_with_the_paths_the_library_knows() {
        let mut c = Connection::open_in_memory().unwrap();
        crate::db::schema::migrate(&mut c).unwrap();
        for (id, title, file) in [("a", "One", Some("/m/one.flac")), ("b", "Two", None)] {
            c.execute(
                "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, file)
                 VALUES (?1, ?2, 'al', 'Album', 'ar', 'Artist', ?1, ?3)",
                rusqlite::params![id, title, file],
            )
            .unwrap();
        }
        let (tracks, paths) = load(&c, &["b".into(), "gone".into(), "a".into()]).unwrap();
        assert_eq!(tracks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths["a"], "/m/one.flac");
    }

    #[test]
    fn a_failed_write_says_why() {
        let dir = std::env::temp_dir().join(format!("platter-plfile-{}", std::process::id()));
        let e = write(&dir.join("missing").join("x.m3u8"), "x").unwrap_err();
        assert!(e.to_string().contains("Couldn't save the playlist file: that folder isn't there any more."), "{e}");
    }

    #[test]
    fn saving_over_a_file_replaces_it_whole_and_leaves_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("platter-plfile-over-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("Road trip.m3u8");
        std::fs::write(&f, "old and longer text").unwrap();
        write(&f, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
