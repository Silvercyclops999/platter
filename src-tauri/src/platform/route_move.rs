//! What an iPhone audio route change means for playback: headphones that
//! leave pause it, and the same ones back within a few seconds (AirPods
//! hopping to a Mac while it only says "connecting", then back) play on.
//! Kept apart from the iOS glue so it's tested everywhere.

use std::time::{Duration, Instant};

/// Headphones that come back this soon after leaving play on.
pub const HOP_BACK: Duration = Duration::from_secs(15);

/// What a route change means for playback.
#[derive(Debug, PartialEq, Eq)]
pub enum RouteMove {
    /// Headphones left: pause, and remember them.
    Left(String),
    /// The headphones that just left came back: play on.
    Back,
    Nothing,
}

/// Decides a route change. `before` is the headphones the previous route
/// played to (`None` when the system didn't say what it was); `left` is the
/// headphones that last left and when.
pub fn route_move(
    reason: usize,
    before: Option<Option<String>>,
    now: Option<String>,
    left: &Option<(String, Instant)>,
    at: Instant,
) -> RouteMove {
    const NEW_DEVICE: usize = 1;
    const OLD_DEVICE_GONE: usize = 2;
    match reason {
        OLD_DEVICE_GONE => match before {
            Some(Some(name)) if now.as_deref() != Some(name.as_str()) => RouteMove::Left(name),
            // Unknown before: a device went away and no headphones are left,
            // so don't carry on out loud. Nothing to play on for.
            None if now.is_none() => RouteMove::Left(String::new()),
            _ => RouteMove::Nothing,
        },
        NEW_DEVICE => match (now, left) {
            (Some(name), Some((gone, when))) if !gone.is_empty() && &name == gone && at.duration_since(*when) <= HOP_BACK => RouteMove::Back,
            _ => RouteMove::Nothing,
        },
        _ => RouteMove::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }
    fn was(v: &str) -> Option<Option<String>> {
        Some(s(v))
    }

    #[test]
    fn headphones_leaving_pause_and_coming_back_soon_plays_on() {
        let t0 = Instant::now();
        assert_eq!(route_move(2, was("AirPods Pro"), None, &None, t0), RouteMove::Left("AirPods Pro".into()));
        let left = Some(("AirPods Pro".to_string(), t0));
        assert_eq!(route_move(1, None, s("AirPods Pro"), &left, t0 + Duration::from_secs(3)), RouteMove::Back);
        // Too late, or other headphones: stay paused.
        assert_eq!(route_move(1, None, s("AirPods Pro"), &left, t0 + Duration::from_secs(60)), RouteMove::Nothing);
        assert_eq!(route_move(1, None, s("Beats"), &left, t0), RouteMove::Nothing);
    }

    #[test]
    fn speaker_and_category_changes_do_nothing() {
        let t0 = Instant::now();
        // The speaker going away isn't headphones leaving.
        assert_eq!(route_move(2, Some(None), None, &None, t0), RouteMove::Nothing);
        // Headphones still in the route (another port left).
        assert_eq!(route_move(2, was("AirPods Pro"), s("AirPods Pro"), &None, t0), RouteMove::Nothing);
        // Category change, override and the like.
        assert_eq!(route_move(3, was("AirPods Pro"), None, &None, t0), RouteMove::Nothing);
    }

    #[test]
    fn an_unknown_previous_route_pauses_off_headphones_only() {
        let t0 = Instant::now();
        assert_eq!(route_move(2, None, None, &None, t0), RouteMove::Left(String::new()));
        assert_eq!(route_move(2, None, s("AirPods Pro"), &None, t0), RouteMove::Nothing);
        // A nameless leave never counts as the same headphones coming back.
        let left = Some((String::new(), t0));
        assert_eq!(route_move(1, None, s("AirPods Pro"), &left, t0), RouteMove::Nothing);
    }
}
