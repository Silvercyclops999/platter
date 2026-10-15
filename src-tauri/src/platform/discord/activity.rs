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

const IMAGE_HOSTS: &[&str] = &["coverartarchive.org", "i.imgur.com", "files.catbox.moe"];
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

/// Which text Discord's member list shows after "Listening to"
/// (`status_display_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusLine {
    /// The app's name.
    #[default]
    App,
    /// The artist (the activity's `state`).
    Artist,
    /// The song title (the activity's `details`).
    Song,
}

impl StatusLine {
    pub fn code(self) -> i64 {
        match self {
            Self::App => 0,
            Self::Artist => 1,
            Self::Song => 2,
        }
    }
}

/// The separate artists in a credit, for "by A, B": featured artists and
/// ";" or "/" lists are split, while names joined by "&" stay whole (that is
/// often one act).
pub fn credit_list(artist: &str) -> String {
    let mut cur = artist.trim().to_string();
    for sep in [" feat. ", " Feat. ", " FEAT. ", " feat ", " ft. ", " Ft. ", " featuring ", " Featuring ", "; ", " / ", "(feat. ", "(ft. "] {
        cur = cur.replace(sep, "\u{1}");
    }
    let mut names: Vec<String> = Vec::new();
    for part in cur.split(['\u{1}', ',']) {
        let n = part.trim().trim_matches(['(', ')', '[', ']']).trim();
        if !n.is_empty() && !names.iter().any(|x| x.eq_ignore_ascii_case(n)) {
            names.push(n.to_string());
        }
    }
    names.join(", ")
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
    /// A listening session this computer hosts.
    pub party: Option<Party>,
}

/// A listening session on Discord: its id and how many are in it. A join
/// secret makes Discord offer a join button; Discord hands it to the
/// joining Platter through the socket (ACTIVITY_JOIN), which needs a
/// listener on the socket that Platter doesn't run yet, so none is sent and
/// the invite goes by link.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Party {
    pub id: String,
    pub size: u32,
    pub max: u32,
    pub join: Option<String>,
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

/// Public cover hosts only (Cover Art Archive, Imgur and catbox.moe).
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
///
/// `with_line` adds `status_display_type`; it's left out once Discord has
/// refused it.
pub fn build(song: Option<&Song>, s: &DiscordSettings, kind: Kind, with_line: bool) -> Option<Value> {
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
    if with_line {
        a["status_display_type"] = s.status_line.code().into();
    }
    if let Some(t) = fit(&song.title) {
        a["details"] = t.into();
    }
    let artists = credit_list(&song.artist);
    if !artists.is_empty()
        && let Some(t) = fit(&format!("by {artists}"))
    {
        a["state"] = t.into();
    }

    let cover = song
        .image
        .as_deref()
        .filter(|u| image_allowed(u))
        .filter(|_| !song.unreleased || s.unreleased_art);
    // The cover alone (the app icon only stands in for a missing one).
    let mut assets = json!({ "large_image": cover.unwrap_or(APP_ICON) });
    let hover = if song.playing || song.album.trim().is_empty() {
        song.album.clone()
    } else {
        format!("{} (paused)", song.album.trim())
    };
    if let Some(t) = fit(&hover) {
        assets["large_text"] = t.into();
    }
    a["assets"] = assets;

    if song.playing && song.duration_ms > 0 {
        a["timestamps"] = json!({
            "start": song.started_at_ms,
            "end": song.started_at_ms + song.duration_ms as i64,
        });
    }
    let party = song.party.as_ref().filter(|p| s.listen_along && !p.id.is_empty() && p.size > 0 && p.max >= p.size);
    if let Some(p) = party {
        a["party"] = json!({ "id": p.id, "size": [p.size, p.max] });
        if let Some(fit) = p.join.as_deref().filter(|j| !j.is_empty() && j.len() <= TEXT_MAX) {
            a["secrets"] = json!({ "join": fit });
        }
    }
    // Discord takes buttons or a join secret, not both.
    if s.find_song
        && !song.unreleased
        && a.get("secrets").is_none()
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
    fn a_hosted_listening_session_shows_as_a_party_when_on() {
        let mut s = song();
        s.party = Some(Party { id: "0123456789abcdef".into(), size: 3, max: 11, join: None });
        let a = build(Some(&s), &on(), Kind::Listening, true).unwrap();
        assert_eq!(a["party"]["id"], "0123456789abcdef");
        assert_eq!(a["party"]["size"], json!([3, 11]));
        assert!(a.get("secrets").is_none());
        assert_eq!(a["buttons"][0]["label"], "Find this song", "no join secret: the button stays");
        // A join secret replaces the buttons (Discord takes one or the other).
        s.party.as_mut().unwrap().join = Some("secret".into());
        let a = build(Some(&s), &on(), Kind::Listening, true).unwrap();
        assert_eq!(a["secrets"]["join"], "secret");
        assert!(a.get("buttons").is_none());
        // The setting turns it off.
        let off = DiscordSettings { listen_along: false, ..on() };
        let a = build(Some(&s), &off, Kind::Listening, true).unwrap();
        assert!(a.get("party").is_none() && a.get("secrets").is_none());
    }

    #[test]
    fn a_playing_song_is_listening_with_cover_progress_and_button() {
        let a = build(Some(&song()), &on(), Kind::Listening, true).unwrap();
        assert_eq!(a["type"], 2);
        assert_eq!(a["details"], "Song");
        assert_eq!(a["state"], "by Band");
        assert_eq!(a["status_display_type"], 0, "the member list names the app by default");
        assert_eq!(a["assets"]["large_text"], "Record");
        assert!(a["assets"]["large_image"].as_str().unwrap().starts_with("https://coverartarchive.org/"));
        assert!(a["assets"].get("small_image").is_none(), "no small image beside a cover");
        assert_eq!(a["timestamps"]["start"], 1_000_000);
        assert_eq!(a["timestamps"]["end"], 1_200_000);
        assert_eq!(a["buttons"][0]["label"], "Find this song");
        assert_eq!(a["buttons"][0]["url"], "https://song.link/i/123");
        assert_eq!(build(Some(&song()), &on(), Kind::Playing, true).unwrap()["type"], 0);
    }

    #[test]
    fn off_private_or_nothing_playing_shows_nothing() {
        assert!(build(None, &on(), Kind::Listening, true).is_none());
        let off = DiscordSettings { enabled: false, ..on() };
        assert!(build(Some(&song()), &off, Kind::Listening, true).is_none());
        let private = DiscordSettings { private_session: true, ..on() };
        assert!(build(Some(&song()), &private, Kind::Listening, true).is_none());
    }

    #[test]
    fn pause_clears_or_shows_paused_without_progress() {
        let paused = Song { playing: false, ..song() };
        assert!(build(Some(&paused), &on(), Kind::Listening, true).is_none());
        let keep = DiscordSettings { clear_on_pause: false, ..on() };
        let a = build(Some(&paused), &keep, Kind::Listening, true).unwrap();
        assert!(a.get("timestamps").is_none());
        assert_eq!(a["assets"]["large_text"], "Record (paused)");
        assert!(a["assets"].get("small_image").is_none());
    }

    #[test]
    fn without_a_cover_the_app_icon_is_the_only_image() {
        let s = Song { image: None, ..song() };
        let a = build(Some(&s), &on(), Kind::Listening, true).unwrap();
        assert_eq!(a["assets"]["large_image"], APP_ICON);
        assert_eq!(a["assets"]["large_text"], "Record");
        assert!(a["assets"].get("small_image").is_none());
    }

    #[test]
    fn status_line_follows_its_setting_and_can_be_left_out() {
        for (line, code) in [(StatusLine::App, 0), (StatusLine::Artist, 1), (StatusLine::Song, 2)] {
            let set = DiscordSettings { status_line: line, ..on() };
            assert_eq!(build(Some(&song()), &set, Kind::Listening, true).unwrap()["status_display_type"], code);
        }
        assert!(build(Some(&song()), &on(), Kind::Listening, false).unwrap().get("status_display_type").is_none());
    }

    #[test]
    fn state_lists_the_artists_after_by() {
        assert_eq!(credit_list("Band feat. Guest"), "Band, Guest");
        assert_eq!(credit_list("A; B / C"), "A, B, C");
        assert_eq!(credit_list("Earth, Wind & Fire"), "Earth, Wind & Fire");
        assert_eq!(credit_list("Simon & Garfunkel"), "Simon & Garfunkel");
        assert_eq!(credit_list("A (feat. B)"), "A, B");
        assert_eq!(credit_list("  "), "");
        let s = Song { artist: "Band ft. Guest".into(), ..song() };
        assert_eq!(build(Some(&s), &on(), Kind::Listening, true).unwrap()["state"], "by Band, Guest");
        let none = Song { artist: String::new(), ..song() };
        assert!(build(Some(&none), &on(), Kind::Listening, true).unwrap().get("state").is_none());
    }

    #[test]
    fn unreleased_songs_follow_their_settings_and_never_get_a_button() {
        let demo = Song { unreleased: true, ..song() };
        let a = build(Some(&demo), &on(), Kind::Listening, true).unwrap();
        assert!(a.get("buttons").is_none());
        assert!(a["assets"]["large_image"].as_str().unwrap().starts_with("https://"));
        let no_art = DiscordSettings { unreleased_art: false, ..on() };
        assert_eq!(build(Some(&demo), &no_art, Kind::Listening, true).unwrap()["assets"]["large_image"], APP_ICON);
        let hidden = DiscordSettings { show_unreleased: false, ..on() };
        assert!(build(Some(&demo), &hidden, Kind::Listening, true).is_none());
        // Released songs are unaffected by the unreleased switches.
        assert!(build(Some(&song()), &hidden, Kind::Listening, true).is_some());
    }

    #[test]
    fn find_song_button_follows_its_setting() {
        let off = DiscordSettings { find_song: false, ..on() };
        assert!(build(Some(&song()), &off, Kind::Listening, true).unwrap().get("buttons").is_none());
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
            let a = build(Some(&s), &on(), Kind::Listening, true).unwrap();
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
