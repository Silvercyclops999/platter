//! Subsonic items as the app's artists, albums, songs and playlists. Every
//! id and image path gets the server's tag (`crate::sid::qualify_any`), so
//! requests about them come back to this server.

use super::types::{Album as SAlbum, Artist as SArtist, ItemDate, Playlist as SPlaylist, Song};
use crate::db::likes::LIKED_RATING;
use crate::db::release_section;
use crate::jellyfin::types::unix_s;
use crate::model::{Album, Artist, Playlist, Track};
use crate::sid::qualify_any;

/// Image path for a cover art id, as the art cache keeps it:
/// `<tag>:/cover/<id>`.
pub fn cover(tag: &str, id: &str) -> String {
    format!("{tag}:/cover/{id}")
}

/// The original file's locator (the song's part key):
/// `<tag>:/stream/<id>.<suffix>`.
pub fn part_key(tag: &str, song_id: &str, suffix: Option<&str>) -> String {
    match suffix {
        Some(s) => format!("{tag}:/stream/{song_id}.{s}"),
        None => format!("{tag}:/stream/{song_id}"),
    }
}

fn text(s: &Option<String>) -> String {
    s.as_deref().map(str::trim).unwrap_or_default().to_string()
}

fn opt_id(tag: &str, id: Option<&str>) -> String {
    id.filter(|i| !i.is_empty()).map(|i| qualify_any(tag, i)).unwrap_or_default()
}

fn art(tag: &str, id: &Option<String>) -> Option<String> {
    id.as_deref().filter(|i| !i.is_empty()).map(|i| cover(tag, i))
}

fn mbid(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(str::to_string)
}

fn genres(list: &[super::types::Named], one: &Option<String>) -> Vec<String> {
    let mut g: Vec<String> = list.iter().map(|n| n.name.trim().to_string()).filter(|n| !n.is_empty()).collect();
    if g.is_empty()
        && let Some(o) = one.as_deref().map(str::trim).filter(|o| !o.is_empty())
    {
        g.push(o.to_string());
    }
    g
}

pub fn artist(tag: &str, a: &SArtist) -> Artist {
    let title = text(&a.name);
    Artist {
        id: qualify_any(tag, &a.id),
        sort_title: a.sort_name.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| title.clone()),
        title,
        thumb: art(tag, &a.cover_art),
        art: None,
        view_count: 0,
        summary: None,
    }
}

fn date(d: &Option<ItemDate>) -> Option<String> {
    let d = d.as_ref()?;
    let y = d.year.filter(|y| *y > 0)?;
    Some(match (d.month, d.day) {
        (Some(m), Some(day)) if m > 0 && day > 0 => format!("{y:04}-{m:02}-{day:02}"),
        (Some(m), _) if m > 0 => format!("{y:04}-{m:02}"),
        _ => format!("{y:04}"),
    })
}

/// The album's artist: id and name.
fn album_artist(tag: &str, a: &SAlbum) -> (String, String) {
    let first = a.artists.first();
    let id = opt_id(tag, a.artist_id.as_deref().or(first.map(|f| f.id.as_str())));
    let name = a
        .display_artist
        .clone()
        .or_else(|| a.artist.clone())
        .filter(|n| !n.trim().is_empty())
        .or_else(|| first.map(|f| f.name.clone()))
        .unwrap_or_default();
    (id, name.trim().to_string())
}

pub fn album(tag: &str, a: &SAlbum) -> Album {
    let (artist_id, artist_title) = album_artist(tag, a);
    let title = text(&a.name);
    // Release types as the server tags them: the first is the main type.
    let mut types: Vec<String> = a.release_types.iter().map(|t| t.trim().to_ascii_lowercase()).filter(|t| !t.is_empty()).collect();
    if a.is_compilation == Some(true) && !types.iter().any(|t| t.eq_ignore_ascii_case("compilation")) {
        types.push("compilation".into());
    }
    // A secondary type alone (a compilation, a live record) is an album of
    // that kind.
    const SECONDARY: [&str; 6] = ["compilation", "live", "remix", "soundtrack", "demo", "mixtape"];
    let (format, subformats) = match types.first() {
        Some(f) if SECONDARY.iter().any(|x| f.contains(x)) => (Some("album".to_string()), types.clone()),
        Some(f) => (Some(f.clone()), types.iter().skip(1).cloned().collect()),
        None => (None, Vec::new()),
    };
    Album {
        id: qualify_any(tag, &a.id),
        sort_title: a.sort_name.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| title.clone()),
        title,
        artist_id,
        artist_title,
        year: a.year.filter(|y| *y > 0),
        thumb: art(tag, &a.cover_art),
        section: release_section(format.as_deref(), &subformats),
        format,
        subformats,
        genres: genres(&a.genres, &a.genre),
        track_count: a.song_count.unwrap_or(a.song.len() as u64) as u32,
        added_at: a.created.as_deref().and_then(unix_s).unwrap_or(0),
        view_count: a.play_count.unwrap_or(0) as u32,
        last_viewed_at: a.played.as_deref().and_then(unix_s),
        tint: None,
        released: date(&a.release_date).or_else(|| date(&a.original_release_date)),
        studio: a.record_labels.first().map(|l| l.name.trim().to_string()).filter(|l| !l.is_empty()),
        matched: None,
        styles: Vec::new(),
        moods: a.moods.clone(),
        mbid: mbid(&a.music_brainz_id),
    }
}

/// Whether an entry is a song (not a video or a folder).
pub fn is_song(s: &Song) -> bool {
    !s.id.is_empty() && s.is_video != Some(true) && s.kind.as_deref().is_none_or(|k| k.eq_ignore_ascii_case("music"))
}

/// A song. A starred song is liked, with the date it was starred.
pub fn track(tag: &str, s: &Song) -> Track {
    let album_artist = s.album_artists.first();
    let artist_title = s
        .display_album_artist
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| album_artist.map(|a| a.name.clone()))
        .or_else(|| s.artist.clone())
        .unwrap_or_default()
        .trim()
        .to_string();
    let artist_id = opt_id(tag, album_artist.map(|a| a.id.as_str()).or(s.artist_id.as_deref()));
    let song_artist = s
        .display_artist
        .clone()
        .or_else(|| s.artist.clone())
        .map(|a| a.trim().to_string())
        .filter(|a| !a.is_empty());
    let track_artist = song_artist.filter(|a| !a.eq_ignore_ascii_case(&artist_title));
    let liked = s.starred.is_some();
    let suffix = s.suffix.as_deref().map(|x| x.trim().to_ascii_lowercase()).filter(|x| !x.is_empty() && x.len() <= 5);
    Track {
        id: qualify_any(tag, &s.id),
        title: text(&s.title),
        album_id: opt_id(tag, s.album_id.as_deref()),
        album_title: text(&s.album),
        artist_id,
        artist_title,
        track_artist,
        track_number: s.track.unwrap_or(0) as u32,
        disc_number: s.disc_number.unwrap_or(1).max(1) as u32,
        duration_ms: s.duration.unwrap_or(0) * 1000,
        thumb: art(tag, &s.cover_art),
        part_key: part_key(tag, &s.id, suffix.as_deref()),
        codec: suffix.clone(),
        container: suffix,
        bitrate: s.bit_rate.map(|b| b as u32).filter(|b| *b > 0),
        bit_depth: s.bit_depth.map(|b| b as u32).filter(|b| *b > 0),
        sample_rate: s.sampling_rate.map(|r| r as u32).filter(|r| *r > 0),
        user_rating: liked.then_some(LIKED_RATING as f32),
        rated_at: s.starred.as_deref().and_then(unix_s),
        view_count: s.play_count.unwrap_or(0) as u32,
        last_viewed_at: s.played.as_deref().and_then(unix_s),
        rating_count: None,
        genres: genres(&s.genres, &s.genre),
        mbid: mbid(&s.music_brainz_id),
        file: s.path.clone().filter(|p| !p.trim().is_empty()),
        other_versions: None,
        song_status: None,
    }
}

pub fn playlist(tag: &str, p: &SPlaylist) -> Playlist {
    Playlist {
        id: qualify_any(tag, &p.id),
        title: text(&p.name),
        smart: false,
        track_count: p.song_count.unwrap_or(p.entry.len() as u64) as u32,
        duration_ms: p.duration.unwrap_or(0) * 1000,
        thumb: art(tag, &p.cover_art),
        summary: p.comment.clone().filter(|c| !c.trim().is_empty()),
        updated_at: p.changed.as_deref().or(p.created.as_deref()).and_then(unix_s),
    }
}

/// A playlist row's id: its index and the song there (`<index>@<song id>`),
/// so an edit can check the row still holds that song.
pub fn row_id(index: usize, song_id: &str) -> String {
    format!("{index}@{song_id}")
}

pub fn parse_row(item_id: &str) -> Option<(usize, &str)> {
    let (i, s) = item_id.split_once('@')?;
    Some((i.parse().ok()?, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Song {
        serde_json::from_value(serde_json::json!({
            "id": "s1", "title": "Song", "album": "Record", "albumId": "al1", "artist": "Band feat. Guest",
            "displayArtist": "Band feat. Guest", "artistId": "ar1", "albumArtists": [{"id": "ar1", "name": "Band"}],
            "track": 3, "discNumber": 2, "duration": 200, "coverArt": "al-al1", "suffix": "FLAC", "bitRate": 900,
            "bitDepth": 24, "samplingRate": 96000, "playCount": 4, "played": "2024-02-29T12:34:56Z",
            "starred": "2024-01-01T00:00:00Z", "musicBrainzId": "mb-rec", "genres": [{"name": "Rock"}],
            "path": "Band/Record/03 Song.flac", "type": "music"
        }))
        .unwrap()
    }

    #[test]
    fn songs_carry_the_tag_and_the_star_date() {
        let t = track("0badf00d", &song());
        assert_eq!(t.id, "0badf00d:s1");
        assert_eq!((t.album_id.as_str(), t.artist_id.as_str()), ("0badf00d:al1", "0badf00d:ar1"));
        assert_eq!(t.thumb.as_deref(), Some("0badf00d:/cover/al-al1"));
        assert_eq!(t.part_key, "0badf00d:/stream/s1.flac");
        assert_eq!(crate::sid::tag_of(&t.part_key), Some("0badf00d"));
        assert_eq!((t.artist_title.as_str(), t.track_artist.as_deref()), ("Band", Some("Band feat. Guest")));
        assert_eq!((t.track_number, t.disc_number, t.duration_ms), (3, 2, 200_000));
        assert_eq!((t.codec.as_deref(), t.bit_depth, t.sample_rate, t.bitrate), (Some("flac"), Some(24), Some(96000), Some(900)));
        assert_eq!((t.user_rating, t.rated_at), (Some(10.0), Some(1_704_067_200)));
        assert_eq!((t.view_count, t.last_viewed_at), (4, Some(1_709_210_096)));
        assert_eq!((t.mbid.as_deref(), t.genres.as_slice()), (Some("mb-rec"), &["Rock".to_string()][..]));
        let mut plain = song();
        plain.starred = None;
        plain.album_artists.clear();
        plain.display_artist = None;
        plain.artist = Some("Band".into());
        let t = track("0badf00d", &plain);
        assert_eq!((t.user_rating, t.rated_at, t.track_artist), (None, None, None));
        assert_eq!(t.artist_id, "0badf00d:ar1");
    }

    #[test]
    fn albums_keep_release_types_and_labels() {
        let a: SAlbum = serde_json::from_value(serde_json::json!({
            "id": "al1", "name": "Record", "artist": "Band", "artistId": "ar1", "coverArt": "al-al1", "songCount": 11,
            "created": "1970-01-02T00:00:00Z", "year": 2001, "releaseTypes": ["Album", "Live"],
            "recordLabels": [{"name": "Label"}], "musicBrainzId": "mb-rel", "releaseDate": {"year": 2001, "month": 5, "day": 1}
        }))
        .unwrap();
        let al = album("0badf00d", &a);
        assert_eq!((al.id.as_str(), al.artist_id.as_str(), al.track_count, al.added_at), ("0badf00d:al1", "0badf00d:ar1", 11, 86_400));
        assert_eq!((al.format.as_deref(), al.subformats.as_slice()), (Some("album"), &["live".to_string()][..]));
        assert_eq!(al.section, crate::model::ReleaseSection::Live);
        assert_eq!((al.released.as_deref(), al.studio.as_deref(), al.mbid.as_deref()), (Some("2001-05-01"), Some("Label"), Some("mb-rel")));
        let mut comp = a.clone();
        comp.release_types.clear();
        comp.is_compilation = Some(true);
        assert_eq!(album("0badf00d", &comp).section, crate::model::ReleaseSection::Compilations);
    }

    #[test]
    fn playlists_and_rows() {
        let p: SPlaylist = serde_json::from_value(serde_json::json!({
            "id": 5, "name": "Mix", "comment": "For the car", "songCount": 2, "duration": 300, "changed": "1970-01-02T00:00:00Z"
        }))
        .unwrap();
        let pl = playlist("0badf00d", &p);
        assert_eq!((pl.id.as_str(), pl.track_count, pl.duration_ms, pl.summary.as_deref()), ("0badf00d:5", 2, 300_000, Some("For the car")));
        assert_eq!(pl.updated_at, Some(86_400));
        assert_eq!(parse_row(&row_id(3, "s:1@x")), Some((3, "s:1@x")));
        assert_eq!(parse_row("nope"), None);
    }

    #[test]
    fn videos_are_not_songs() {
        let mut s = song();
        assert!(is_song(&s));
        s.is_video = Some(true);
        assert!(!is_song(&s));
    }
}
