//! Jellyfin items as the app's artists, albums, songs and playlists. Every
//! id and image path gets the server's tag (`crate::sid::qualify_any`), so
//! requests about them come back to this server.

use super::types::{Item, MediaStream, ticks_ms, unix_s};
use crate::db::likes::LIKED_RATING;
use crate::model::{Album, Artist, Playlist, ReleaseSection, Track};
use crate::sid::qualify_any;

/// Image path for an item's picture of `kind` (`Primary`, `Backdrop/0`), as
/// the art cache keeps it: `<tag>:/Items/<id>/Images/Primary?tag=<image tag>`.
pub fn image(tag: &str, item_id: &str, kind: &str, image_tag: &str) -> String {
    format!("{tag}:/Items/{item_id}/Images/{kind}?tag={image_tag}")
}

fn primary(tag: &str, it: &Item) -> Option<String> {
    it.image_tags.get("Primary").filter(|t| !t.is_empty()).map(|t| image(tag, &it.id, "Primary", t))
}

fn text(s: &Option<String>) -> String {
    s.as_deref().map(str::trim).unwrap_or_default().to_string()
}

fn sort(it: &Item) -> String {
    it.sort_name.clone().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| text(&it.name))
}

fn opt_id(tag: &str, id: Option<&str>) -> String {
    id.filter(|i| !i.is_empty()).map(|i| qualify_any(tag, i)).unwrap_or_default()
}

/// The album artist: its id and name.
fn album_artist(tag: &str, it: &Item) -> (String, String) {
    let first = it.album_artists.first().or_else(|| it.artist_items.first());
    let id = opt_id(tag, first.and_then(|a| a.id.as_deref()));
    let name = it
        .album_artist
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| first.map(|a| a.name.clone()))
        .unwrap_or_default();
    (id, name)
}

fn mbid(it: &Item, key: &str) -> Option<String> {
    it.provider_ids
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn plays(it: &Item) -> (u32, Option<i64>) {
    match &it.user_data {
        Some(u) => (u.play_count, u.last_played_date.as_deref().and_then(unix_s)),
        None => (0, None),
    }
}

pub fn artist(tag: &str, it: &Item) -> Artist {
    Artist {
        id: qualify_any(tag, &it.id),
        title: text(&it.name),
        sort_title: sort(it),
        thumb: primary(tag, it),
        art: it.backdrop_image_tags.first().map(|t| image(tag, &it.id, "Backdrop/0", t)),
        view_count: plays(it).0,
        summary: it.overview.clone().filter(|s| !s.trim().is_empty()),
        mbid: mbid(it, "MusicBrainzArtist"),
    }
}

pub fn album(tag: &str, it: &Item) -> Album {
    let (artist_id, artist_title) = album_artist(tag, it);
    let (view_count, last_viewed_at) = plays(it);
    Album {
        id: qualify_any(tag, &it.id),
        title: text(&it.name),
        sort_title: sort(it),
        artist_id,
        artist_title,
        year: it.production_year,
        thumb: primary(tag, it),
        // Jellyfin keeps no release types; the file tag scan fills them in.
        format: None,
        subformats: Vec::new(),
        section: ReleaseSection::Albums,
        genres: it.genres.clone(),
        track_count: it.child_count.or(it.song_count).unwrap_or(0),
        added_at: it.date_created.as_deref().and_then(unix_s).unwrap_or(0),
        view_count,
        last_viewed_at,
        tint: None,
        released: it.premiere_date.as_deref().and_then(|d| d.get(..10)).map(str::to_string),
        studio: it.studios.first().map(|s| s.name.clone()).filter(|s| !s.trim().is_empty()),
        matched: None,
        styles: Vec::new(),
        moods: Vec::new(),
        mbid: mbid(it, "MusicBrainzAlbum"),
    }
}

/// The file's extension: from its path, else the container Jellyfin names
/// (the first of a list like `mov,mp4,m4a`).
fn extension(it: &Item) -> Option<String> {
    let source = it.media_sources.first();
    let path = source.and_then(|s| s.path.as_deref()).or(it.path.as_deref());
    let from_path = path
        .and_then(|p| p.rsplit(['/', '\\']).next())
        .and_then(|f| f.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()))
        .filter(|e| !e.is_empty() && e.len() <= 5 && e.bytes().all(|b| b.is_ascii_alphanumeric()));
    from_path.or_else(|| {
        source
            .and_then(|s| s.container.as_deref())
            .or(it.container.as_deref())
            .and_then(|c| c.split(',').next())
            .map(|c| c.trim().to_ascii_lowercase())
            .filter(|c| !c.is_empty())
    })
}

/// Where the original file streams from (the song's part key): the static
/// stream path with the file's extension, under the server's tag.
pub fn part_key(tag: &str, item_id: &str, ext: Option<&str>) -> String {
    match ext {
        Some(e) => format!("{tag}:/Audio/{item_id}/stream.{e}"),
        None => format!("{tag}:/Audio/{item_id}/stream"),
    }
}

fn audio_stream(it: &Item) -> Option<&MediaStream> {
    it.media_sources
        .first()?
        .media_streams
        .iter()
        .find(|s| s.kind.as_deref().is_some_and(|k| k.eq_ignore_ascii_case("audio")))
}

/// A song. `liked_at` is when it was first seen liked (Jellyfin keeps no
/// date for favourites).
pub fn track(tag: &str, it: &Item, liked_at: impl Fn(&str) -> Option<i64>) -> Track {
    let (artist_id, artist_title) = album_artist(tag, it);
    let names: Vec<&str> = it.artists.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let track_artist = (!names.is_empty() && !(names.len() == 1 && names[0].eq_ignore_ascii_case(&artist_title)))
        .then(|| names.join(", "));
    let id = qualify_any(tag, &it.id);
    let liked = it.user_data.as_ref().is_some_and(|u| u.is_favorite);
    let (view_count, last_viewed_at) = plays(it);
    let stream = audio_stream(it);
    let source = it.media_sources.first();
    let ext = extension(it);
    let album_thumb = match (&it.album_id, &it.album_primary_image_tag) {
        (Some(a), Some(t)) if !a.is_empty() && !t.is_empty() => Some(image(tag, a, "Primary", t)),
        _ => None,
    };
    Track {
        title: text(&it.name),
        album_id: opt_id(tag, it.album_id.as_deref()),
        album_title: text(&it.album),
        artist_id,
        artist_title,
        track_artist,
        track_number: it.index_number.unwrap_or(0),
        disc_number: it.parent_index_number.unwrap_or(1).max(1),
        duration_ms: it.run_time_ticks.map(ticks_ms).unwrap_or(0),
        thumb: album_thumb.or_else(|| primary(tag, it)),
        part_key: part_key(tag, &it.id, ext.as_deref()),
        codec: stream.and_then(|s| s.codec.clone()).map(|c| c.to_ascii_lowercase()),
        container: ext,
        bitrate: source
            .and_then(|s| s.bitrate)
            .or_else(|| stream.and_then(|s| s.bit_rate))
            .map(|b| (b / 1000) as u32),
        bit_depth: stream.and_then(|s| s.bit_depth),
        sample_rate: stream.and_then(|s| s.sample_rate),
        user_rating: liked.then_some(LIKED_RATING as f32),
        rated_at: if liked { liked_at(&id) } else { None },
        view_count,
        last_viewed_at,
        rating_count: None,
        genres: it.genres.clone(),
        mbid: mbid(it, "MusicBrainzTrack"),
        file: source.and_then(|s| s.path.clone()).or_else(|| it.path.clone()),
        other_versions: None,
        song_status: None,
        file_release: None,
        id,
    }
}

pub fn playlist(tag: &str, it: &Item) -> Playlist {
    Playlist {
        id: qualify_any(tag, &it.id),
        title: text(&it.name),
        smart: false,
        track_count: it.child_count.unwrap_or(0),
        duration_ms: it.cumulative_run_time_ticks.or(it.run_time_ticks).map(ticks_ms).unwrap_or(0),
        thumb: primary(tag, it),
        summary: it.overview.clone().filter(|s| !s.trim().is_empty()),
        updated_at: it.date_last_saved.as_deref().or(it.date_created.as_deref()).and_then(unix_s),
    }
}

/// An audio playlist (or one that doesn't say).
pub fn is_audio_playlist(it: &Item) -> bool {
    it.media_type.as_deref().is_none_or(|m| m.eq_ignore_ascii_case("audio"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song() -> Item {
        serde_json::from_value(serde_json::json!({
            "Id": "6f1b2c3d4e5f60718293a4b5c6d7e8f9",
            "Name": "Song",
            "Type": "Audio",
            "AlbumId": "aa11",
            "Album": "Record",
            "AlbumArtist": "Band",
            "AlbumArtists": [{"Name": "Band", "Id": "bb22"}],
            "Artists": ["Band", "Guest"],
            "AlbumPrimaryImageTag": "img1",
            "IndexNumber": 3,
            "ParentIndexNumber": 2,
            "RunTimeTicks": 2_000_000_000u64,
            "ProviderIds": {"MusicBrainzTrack": "mb-track"},
            "Genres": ["Rock"],
            "UserData": {"IsFavorite": true, "PlayCount": 4, "LastPlayedDate": "2024-02-29T12:34:56Z"},
            "MediaSources": [{"Container": "flac", "Bitrate": 900_000, "Path": "/music/Band/Record/03 Song.FLAC",
                "MediaStreams": [{"Type": "Audio", "Codec": "FLAC", "BitDepth": 24, "SampleRate": 96000}]}]
        }))
        .unwrap()
    }

    #[test]
    fn songs_carry_the_tag_everywhere() {
        let t = track("0badf00d", &song(), |_| Some(77));
        assert_eq!(t.id, "0badf00d:6f1b2c3d4e5f60718293a4b5c6d7e8f9");
        assert_eq!(t.album_id, "0badf00d:aa11");
        assert_eq!(t.artist_id, "0badf00d:bb22");
        assert_eq!(t.thumb.as_deref(), Some("0badf00d:/Items/aa11/Images/Primary?tag=img1"));
        assert_eq!(t.part_key, "0badf00d:/Audio/6f1b2c3d4e5f60718293a4b5c6d7e8f9/stream.flac");
        assert_eq!(crate::sid::tag_of(&t.part_key), Some("0badf00d"));
        assert_eq!(t.track_artist.as_deref(), Some("Band, Guest"));
        assert_eq!((t.track_number, t.disc_number, t.duration_ms), (3, 2, 200_000));
        assert_eq!((t.codec.as_deref(), t.bit_depth, t.sample_rate, t.bitrate), (Some("flac"), Some(24), Some(96000), Some(900)));
        assert_eq!(t.user_rating, Some(10.0));
        assert_eq!(t.rated_at, Some(77));
        assert_eq!((t.view_count, t.last_viewed_at), (4, Some(1_709_210_096)));
        assert_eq!(t.mbid.as_deref(), Some("mb-track"));
    }

    #[test]
    fn a_song_by_its_album_artist_alone_has_no_track_artist() {
        let mut it = song();
        it.artists = vec!["band".into()];
        it.user_data = None;
        let t = track("0badf00d", &it, |_| Some(1));
        assert_eq!(t.track_artist, None);
        assert_eq!((t.user_rating, t.rated_at), (None, None));
    }

    #[test]
    fn albums_artists_and_playlists() {
        let a: Item = serde_json::from_value(serde_json::json!({
            "Id": "aa11", "Name": "Record", "SortName": "record", "Type": "MusicAlbum", "ProductionYear": 2001,
            "AlbumArtist": "Band", "AlbumArtists": [{"Name": "Band", "Id": "bb22"}], "ImageTags": {"Primary": "p"},
            "ChildCount": 11, "DateCreated": "1970-01-02T00:00:00Z", "PremiereDate": "2001-05-01T00:00:00.0000000Z",
            "Studios": [{"Name": "Label"}], "ProviderIds": {"MusicBrainzAlbum": "mb-rel"}
        }))
        .unwrap();
        let al = album("0badf00d", &a);
        assert_eq!((al.id.as_str(), al.artist_id.as_str(), al.track_count, al.added_at), ("0badf00d:aa11", "0badf00d:bb22", 11, 86_400));
        assert_eq!((al.released.as_deref(), al.studio.as_deref(), al.mbid.as_deref()), (Some("2001-05-01"), Some("Label"), Some("mb-rel")));
        assert_eq!(al.section, ReleaseSection::Albums);
        let r: Item = serde_json::from_value(serde_json::json!({
            "Id": "bb22", "Name": "Band", "Type": "MusicArtist", "Overview": "Bio", "BackdropImageTags": ["bd"]
        }))
        .unwrap();
        let ar = artist("0badf00d", &r);
        assert_eq!(ar.art.as_deref(), Some("0badf00d:/Items/bb22/Images/Backdrop/0?tag=bd"));
        assert_eq!((ar.thumb, ar.summary.as_deref()), (None, Some("Bio")));
        let p: Item = serde_json::from_value(serde_json::json!({
            "Id": "pp", "Name": "Mix", "Type": "Playlist", "MediaType": "Audio", "ChildCount": 2, "CumulativeRunTimeTicks": 3_000_000_000u64
        }))
        .unwrap();
        let pl = playlist("0badf00d", &p);
        assert_eq!((pl.id.as_str(), pl.track_count, pl.duration_ms), ("0badf00d:pp", 2, 300_000));
        assert!(is_audio_playlist(&p));
    }
}
