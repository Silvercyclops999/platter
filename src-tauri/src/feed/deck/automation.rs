//! `platter://` command links, for launchers, shortcuts and scripts:
//!
//! - `platter://play`, `platter://pause`, `platter://toggle`,
//!   `platter://next`, `platter://previous`
//! - `platter://play?playlist=<name or id>` (`&shuffle=1` to shuffle)
//! - `platter://search?q=<text>`
//! - `platter://open/<page>` (see [`page_route`])
//!
//! They run only while Allow automation is on. Any web page can open a
//! link, so there is no `like` link: liking stays on the paired local API.
//! Links to music
//! (`platter://album/...` and the rest) are sharing's, not these.

use super::Command;
use crate::model::Playlist;

/// The pages `open` knows, and the window route each one is.
const PAGES: &[(&str, &str)] = &[
    ("home", "home"),
    ("search", "search"),
    ("library", "library"),
    ("liked", "liked"),
    ("settings", "settings"),
    ("stats", "stats"),
    ("recents", "recents"),
    ("bookmarks", "bookmarks"),
    ("lyrics", "lyrics"),
    ("release-radar", "releaseRadar"),
    ("whats-new", "whatsNew"),
];

/// The window route for a page name (`home`, `liked`, `release-radar`...).
pub fn page_route(page: &str) -> Option<serde_json::Value> {
    let p = page.trim().to_ascii_lowercase().replace([' ', '_'], "-");
    PAGES.iter().find(|(name, _)| *name == p).map(|(_, route)| serde_json::json!({ "name": route }))
}

/// Read a command link. None when it isn't one (it may be a link to music).
pub fn parse_link(raw: &str) -> Option<Command> {
    let u = url::Url::parse(raw.trim()).ok()?;
    if u.scheme() != "platter" {
        return None;
    }
    // platter://next has the command as the host; platter:next and
    // platter:///next have it in the path.
    let mut parts: Vec<String> = u.host_str().into_iter().map(str::to_owned).collect();
    parts.extend(u.path().split('/').filter(|s| !s.is_empty()).map(|s| percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()));
    let query = |k: &str| u.query_pairs().find(|(key, _)| key == k).map(|(_, v)| v.into_owned());
    let yes = |k: &str| query(k).is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"));
    let first = parts.first()?.to_ascii_lowercase();
    Some(match (first.as_str(), parts.len()) {
        ("play", 1) => match query("playlist") {
            Some(p) => Command::PlayPlaylist { playlist: p, shuffle: yes("shuffle") },
            None => Command::Play,
        },
        ("pause", 1) => Command::Pause,
        ("toggle" | "play-pause", 1) => Command::PlayPause,
        ("next", 1) => Command::Next,
        ("previous" | "prev", 1) => Command::Previous,
        ("search", 1) => Command::Search { query: query("q").unwrap_or_default() },
        ("open", 2) => Command::Open { page: parts[1].clone() },
        _ => return None,
    })
}

/// Whether a launch argument is a command link (the window stays where it is).
pub fn is_command_link(arg: &str) -> bool {
    parse_link(arg).is_some_and(|c| !matches!(c, Command::Search { .. } | Command::Open { .. }))
}

/// The playlist a command means: its id, else a name matching exactly
/// (ignoring case), else the one whose name starts with it, else one
/// containing it. Ties go to the shortest name.
pub fn pick_playlist<'a>(lists: &'a [Playlist], query: &str) -> Option<&'a Playlist> {
    let q = query.trim();
    if q.is_empty() {
        return None;
    }
    if let Some(p) = lists.iter().find(|p| p.id == q || crate::sid::resolve(&p.id) == q) {
        return Some(p);
    }
    let lower = q.to_lowercase();
    let best = |f: &dyn Fn(&str) -> bool| lists.iter().filter(|p| f(&p.title.to_lowercase())).min_by_key(|p| p.title.chars().count());
    best(&|t| t == lower).or_else(|| best(&|t| t.starts_with(&lower))).or_else(|| best(&|t| t.contains(&lower)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pl(id: &str, title: &str) -> Playlist {
        serde_json::from_value(serde_json::json!({ "id": id, "title": title, "smart": false, "trackCount": 1, "durationMs": 1 })).unwrap()
    }

    #[test]
    fn links_read_as_commands() {
        assert_eq!(parse_link("platter://next"), Some(Command::Next));
        assert_eq!(parse_link("platter:///pause"), Some(Command::Pause));
        assert_eq!(parse_link("platter:toggle"), Some(Command::PlayPause));
        assert_eq!(parse_link("PLATTER://Previous"), Some(Command::Previous));
        assert_eq!(parse_link("platter://play"), Some(Command::Play));
        assert_eq!(parse_link("platter://like"), None, "a web page could like songs");
        assert_eq!(
            parse_link("platter://play?playlist=Road%20Trip&shuffle=1"),
            Some(Command::PlayPlaylist { playlist: "Road Trip".into(), shuffle: true })
        );
        assert_eq!(parse_link("platter://search?q=night+drive"), Some(Command::Search { query: "night drive".into() }));
        assert_eq!(parse_link("platter://open/release-radar"), Some(Command::Open { page: "release-radar".into() }));
        // Links to music and invites aren't commands.
        for other in ["platter://album/123", "platter://playlist/9", "platter://join/abc", "https://next", "platter://next/extra", "nonsense"] {
            assert_eq!(parse_link(other), None, "{other}");
        }
        assert!(is_command_link("platter://next") && !is_command_link("platter://open/home") && !is_command_link("--autostart"));
    }

    #[test]
    fn pages_map_to_window_routes() {
        assert_eq!(page_route("Liked"), Some(serde_json::json!({ "name": "liked" })));
        assert_eq!(page_route("release radar"), Some(serde_json::json!({ "name": "releaseRadar" })));
        assert_eq!(page_route("album"), None);
    }

    #[test]
    fn playlists_by_id_then_name() {
        let lists = [pl("7", "Road Trip Extended"), pl("8", "Road Trip"), pl("9", "Gym"), pl("10", "Old road songs")];
        assert_eq!(pick_playlist(&lists, "9").unwrap().title, "Gym");
        assert_eq!(pick_playlist(&lists, "road trip").unwrap().id, "8", "exact name first");
        assert_eq!(pick_playlist(&lists, "road").unwrap().id, "8", "then the shortest starting with it");
        assert_eq!(pick_playlist(&lists, "songs").unwrap().id, "10");
        assert!(pick_playlist(&lists, "jazz").is_none() && pick_playlist(&lists, " ").is_none());
    }
}
