//! Pairing without a code: the Stream Deck plugin asks (`POST
//! /v1/pair/request`), Platter brings its window up with one prompt, and the
//! person allows it or not. The plugin collects the answer with the request's
//! id (`POST /v1/pair/status`), which only it knows.
//!
//! One prompt at a time, at least [`GAP`] between two asks, and a prompt left
//! unanswered for [`TTL`] goes away. Nothing here ever allows on its own: a
//! token is only made by [`Asks::answer`] with `allow`, which only the window
//! calls.

use std::time::{Duration, Instant};

use super::pairing::hex;

/// The least time between two asks.
pub const GAP: Duration = Duration::from_secs(5);
/// How long a prompt waits for an answer, and an answer for the plugin.
pub const TTL: Duration = Duration::from_secs(120);

/// Why an ask was turned away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskError {
    /// A prompt is already showing.
    Busy,
    /// Asked again within [`GAP`].
    TooSoon,
}

/// What the plugin hears back for its request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskState {
    Waiting,
    /// Allowed: the token, handed over once.
    Allowed(String),
    Denied,
    /// Not answered in time, already collected, or never asked.
    Gone,
}

#[derive(Debug, Clone)]
struct Open {
    id: String,
    at: Instant,
}

#[derive(Debug, Clone)]
enum Answer {
    Allowed(String),
    Denied,
}

#[derive(Debug, Default)]
pub struct Asks {
    open: Option<Open>,
    /// The last answer, until the plugin collects it.
    answered: Option<(String, Answer, Instant)>,
    last: Option<Instant>,
}

impl Asks {
    fn expire(&mut self, now: Instant) {
        if self.open.as_ref().is_some_and(|o| now.saturating_duration_since(o.at) >= TTL) {
            self.open = None;
        }
        if self.answered.as_ref().is_some_and(|(_, _, at)| now.saturating_duration_since(*at) >= TTL) {
            self.answered = None;
        }
    }

    /// Open a prompt; its id is the plugin's to keep.
    pub fn open(&mut self, now: Instant) -> Result<String, AskError> {
        self.expire(now);
        if self.open.is_some() {
            return Err(AskError::Busy);
        }
        if self.last.is_some_and(|l| now.saturating_duration_since(l) < GAP) {
            return Err(AskError::TooSoon);
        }
        let id = hex(&rand::random::<[u8; 16]>());
        self.open = Some(Open { id: id.clone(), at: now });
        self.last = Some(now);
        Ok(id)
    }

    /// The prompt showing now, if any.
    pub fn current(&mut self, now: Instant) -> Option<String> {
        self.expire(now);
        self.open.as_ref().map(|o| o.id.clone())
    }

    /// The person answered the prompt `id`. With `allow`, `issue` makes the
    /// token. False when that prompt isn't showing any more.
    pub fn answer<E>(&mut self, id: &str, allow: bool, now: Instant, issue: impl FnOnce() -> Result<String, E>) -> Result<bool, E> {
        self.expire(now);
        if self.open.as_ref().is_none_or(|o| o.id != id) {
            return Ok(false);
        }
        let answer = if allow { Answer::Allowed(issue()?) } else { Answer::Denied };
        self.open = None;
        // The next ask waits from the answer, too.
        self.last = Some(now);
        self.answered = Some((id.to_owned(), answer, now));
        Ok(true)
    }

    /// What the plugin with request `id` hears. A token is handed over once.
    pub fn status(&mut self, id: &str, now: Instant) -> AskState {
        self.expire(now);
        if id.is_empty() {
            return AskState::Gone;
        }
        if self.open.as_ref().is_some_and(|o| o.id == id) {
            return AskState::Waiting;
        }
        match &self.answered {
            Some((a, Answer::Allowed(_), _)) if a == id => match self.answered.take() {
                Some((_, Answer::Allowed(t), _)) => AskState::Allowed(t),
                _ => AskState::Gone,
            },
            Some((a, Answer::Denied, _)) if a == id => AskState::Denied,
            _ => AskState::Gone,
        }
    }
}
