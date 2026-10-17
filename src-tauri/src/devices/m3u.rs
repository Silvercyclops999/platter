//! M3U8 playlist files for the device: UTF-8, one song per line, paths
//! relative to the playlist file (which sits at the top of the music
//! folder), with `/` between folders as Rockbox and most players read them.

/// One song line.
pub struct Line<'a> {
    pub rel: &'a str,
    pub duration_ms: u64,
    pub artist: &'a str,
    pub title: &'a str,
}

fn one_line(s: &str) -> String {
    s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect()
}

pub fn write(lines: &[Line<'_>]) -> String {
    let mut out = String::from("#EXTM3U\n");
    for l in lines {
        let secs = l.duration_ms.div_ceil(1000);
        let label = match (l.artist.trim(), l.title.trim()) {
            ("", t) => one_line(t),
            (a, t) => format!("{} - {}", one_line(a), one_line(t)),
        };
        out.push_str(&format!("#EXTINF:{secs},{label}\n{}\n", l.rel.replace('\\', "/")));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlists_use_relative_paths_and_extinf() {
        let text = write(&[
            Line { rel: "Artist/Album/01 One.mp3", duration_ms: 61_200, artist: "Artist", title: "One" },
            Line { rel: "Other/Album/02 Two.mp3", duration_ms: 0, artist: "", title: "Two\nlines" },
        ]);
        assert_eq!(
            text,
            "#EXTM3U\n#EXTINF:62,Artist - One\nArtist/Album/01 One.mp3\n#EXTINF:0,Two lines\nOther/Album/02 Two.mp3\n"
        );
    }

    #[test]
    fn an_empty_playlist_is_just_the_header() {
        assert_eq!(write(&[]), "#EXTM3U\n");
    }
}
