//! What Discord shows, from the playing song and the settings (pure).
//!
//! Only text from the library (title, artist, album), timestamps, and public
//! URLs on a short list of hosts ever go into an activity. Plex art paths,
//! server addresses and tokens never do: images are checked against
//! [`image_allowed`] and anything else falls back to the app icon.

use serde_json::{Value, json};

use super::DiscordSettings;

/// The app icon's asset key in the Discord application (uploaded under Rich
/// Presence, Art Assets).
pub const APP_ICON: &str = "platter";

/// Discord's text fields take 2 to 128 characters.
const TEXT_MIN: usize = 2;
const TEXT_MAX: usize = 128;
/// Image keys and URLs longer than this are refused.
const IMAGE_MAX: usize = 256;
const BUTTON_URL_MAX: usize = 512;

const IMAGE_HOSTS: &[&str] = &["coverartarchive.org", "i.imgur.com"];
const BUTTON_HOSTS: &[&str] = &["song.link", "album.link"];

/// Listening (2), or Playing (0) when the client refuses Listening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Listening,
    Playing,
}

impl Kind {
    pub fn code(self) -> i64 {
        match self {
            Self::Listening => 2,
            Self::Playing => 0,
        }
    }
}

/// The song as Discord will describe it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Song {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: u64,
    pub unreleased: bool,
    /// Playing (false: paused).
    pub playing: bool,
    /// Unix ms at which the song's position was 0 (while playing).
    pub started_at_ms: i64,
    /// A public cover URL, when one was found.
    pub image: Option<String>,
    /// The song.link page, for released songs.
    pub link: Option<String>,
}

fn host_ok(url: &str, hosts: &[&str], max: usize) -> bool {
    if url.len() > max {
        return false;
    }
    let Ok(u) = url::Url::parse(url) else { return false };
    u.scheme() == "https"
        && u.query().is_none()
        && u.username().is_empty()
        && u.port().is_none()
        && u.host_str().is_some_and(|h| hosts.contains(&h))
}

/// Public cover hosts only (Cover Art Archive and Imgur).
pub fn image_allowed(url: &str) -> bool {
    host_ok(url, IMAGE_HOSTS, IMAGE_MAX)
}

pub fn button_allowed(url: &str) -> bool {
    host_ok(url, BUTTON_HOSTS, BUTTON_URL_MAX)
}

/// Trim to Discord's limits. One-character text gets a blank braille cell
/// so a song called "X" still shows.
pub fn fit(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut out = if s.chars().count() > TEXT_MAX {
        s.chars().take(TEXT_MAX - 1).collect::<String>() + "…"
    } else {
        s.to_owned()
    };
    while out.chars().count() < TEXT_MIN {
        out.push('\u{2800}');
    }
    Some(out)
}

/// The activity to show, or None to show nothing.
pub fn build(song: Option<&Song>, s: &DiscordSettings, kind: Kind) -> Option<Value> {
    if !s.enabled || s.private_session {
        return None;
    }
    let song = song?;
    if song.unreleased && !s.show_unreleased {
        return None;
    }
    if !song.playing && s.clear_on_pause {
        return None;
    }
    let mut a = json!({ "type": kind.code() });
    if let Some(t) = fit(&song.title) {
        a["details"] = t.into();
    }
    if let Some(t) = fit(&song.artist) {
        a["state"] = t.into();
    }

    let cover = song
        .image
        .as_deref()
        .filter(|u| image_allowed(u))
        .filter(|_| !song.unreleased || s.unreleased_art);
    let mut assets = json!({ "large_image": cover.unwrap_or(APP_ICON) });
    if let Some(t) = fit(&song.album) {
        assets["large_text"] = t.into();
    }
    if cover.is_some() || !song.playing {
        assets["small_image"] = APP_ICON.into();
        assets["small_text"] = if song.playing { "Platter" } else { "Paused" }.into();
    }
    a["assets"] = assets;

    if song.playing && song.duration_ms > 0 {
        a["timestamps"] = json!({
            "start": song.started_at_ms,
            "end": song.started_at_ms + song.duration_ms as i64,
        });
    }
    if s.find_song
        && !song.unreleased
        && let Some(url) = song.link.as_deref().filter(|u| button_allowed(u))
    {
        a["buttons"] = json!([{ "label": "Find this song", "url": url }]);
    }
    Some(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> DiscordSettings {
        DiscordSettings::default()
    }

    fn song() -> Song {
        Song {
            title: "Song".into(),
            artist: "Band".into(),
            album: "Record".into(),
            duration_ms: 200_000,
            playing: true,
            started_at_ms: 1_000_000,
            image: Some("https://coverartarchive.org/release/0b1e2a3c-0000-4000-8000-000000000001/front-500".into()),
            link: Some("https://song.link/i/123".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_playing_song_is_listening_with_cover_progress_and_button() {
        let a = build(Some(&song()), &on(), Kind::Listening).unwrap();
        assert_eq!(a["type"], 2);
        assert_eq!(a["details"], "Song");
        assert_eq!(a["state"], "Band");
        assert_eq!(a["assets"]["large_text"], "Record");
        assert!(a["assets"]["large_image"].as_str().unwrap().starts_with("https://coverartarchive.org/"));
        assert_eq!(a["assets"]["small_image"], APP_ICON);
        assert_eq!(a["timestamps"]["start"], 1_000_000);
        assert_eq!(a["timestamps"]["end"], 1_200_000);
        assert_eq!(a["buttons"][0]["label"], "Find this song");
        assert_eq!(a["buttons"][0]["url"], "https://song.link/i/123");
        assert_eq!(build(Some(&song()), &on(), Kind::Playing).unwrap()["type"], 0);
    }

    #[test]
    fn off_private_or_nothing_playing_shows_nothing() {
        assert!(build(None, &on(), Kind::Listening).is_none());
        let off = DiscordSettings { enabled: false, ..on() };
        assert!(build(Some(&song()), &off, Kind::Listening).is_none());
        let private = DiscordSettings { private_session: true, ..on() };
        assert!(build(Some(&song()), &private, Kind::Listening).is_none());
    }

    #[test]
    fn pause_clears_or_shows_paused_without_progress() {
        let paused = Song { playing: false, ..song() };
        assert!(build(Some(&paused), &on(), Kind::Listening).is_none());
        let keep = DiscordSettings { clear_on_pause: false, ..on() };
        let a = build(Some(&paused), &keep, Kind::Listening).unwrap();
        assert!(a.get("timestamps").is_none());
        assert_eq!(a["assets"]["small_text"], "Paused");
    }

    #[test]
    fn unreleased_songs_follow_their_settings_and_never_get_a_button() {
        let demo = Song { unreleased: true, ..song() };
        let a = build(Some(&demo), &on(), Kind::Listening).unwrap();
        assert!(a.get("buttons").is_none());
        assert!(a["assets"]["large_image"].as_str().unwrap().starts_with("https://"));
        let no_art = DiscordSettings { unreleased_art: false, ..on() };
        assert_eq!(build(Some(&demo), &no_art, Kind::Listening).unwrap()["assets"]["large_image"], APP_ICON);
        let hidden = DiscordSettings { show_unreleased: false, ..on() };
        assert!(build(Some(&demo), &hidden, Kind::Listening).is_none());
        // Released songs are unaffected by the unreleased switches.
        assert!(build(Some(&song()), &hidden, Kind::Listening).is_some());
    }

    #[test]
    fn find_song_button_follows_its_setting() {
        let off = DiscordSettings { find_song: false, ..on() };
        assert!(build(Some(&song()), &off, Kind::Listening).unwrap().get("buttons").is_none());
    }

    #[test]
    fn only_public_hosts_reach_discord() {
        for bad in [
            "http://coverartarchive.org/release/x/front-500",
            "https://192.0.2.10:32400/library/metadata/1/thumb?X-Plex-Token=secret",
            "https://abc.plex.direct:32400/photo/:/transcode?url=x",
            "https://i.imgur.com/a.jpg?X-Plex-Token=secret",
            "https://i.imgur.com.evil.example/a.jpg",
            "/library/metadata/1/thumb/2",
        ] {
            assert!(!image_allowed(bad), "{bad}");
            let s = Song { image: Some(bad.into()), link: Some(bad.into()), ..song() };
            let a = build(Some(&s), &on(), Kind::Listening).unwrap();
            assert_eq!(a["assets"]["large_image"], APP_ICON);
            assert!(a.get("buttons").is_none());
        }
        assert!(image_allowed("https://i.imgur.com/AbC123.jpg"));
        assert!(button_allowed("https://album.link/i/5"));
    }

    #[test]
    fn text_fits_discords_limits() {
        assert_eq!(fit("  "), None);
        assert_eq!(fit("X").unwrap().chars().count(), 2);
        let long = "a".repeat(300);
        let f = fit(&long).unwrap();
        assert_eq!(f.chars().count(), 128);
        assert!(f.ends_with('…'));
        assert_eq!(fit(&"é".repeat(128)).unwrap().chars().count(), 128);
    }
}
