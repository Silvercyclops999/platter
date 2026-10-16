//! Share links: `platter://share/<token>`, a link or QR code the host
//! gives someone so their Platter can ask for the host's music without
//! typing a Plex username.
//!
//! The token is listen along's invite token (`listen::invite`): random, an
//! expiry, and the relay when it isn't the default. Opening it, the friend's
//! Platter sends its Plex username to the host, sealed for the link's own
//! channel (`share up`); the host's Platter shares the music the link was
//! made with and answers on `share down`. One person, once: a second answer
//! is turned away. The relay sees random topics and ciphertext only. The
//! host has to have Platter open while the link is used.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio::time::Instant;

use super::{ShareOptions, ShareStatus, Sharing};
use crate::error::{AppError, AppResult};
use crate::remote::peer::auth::{hex, now_ms};
use crate::social::listen::invite::{self, Token};
use crate::social::listen::relay::{Relay, RelayError, RelayEvent, RelayFactory};
use crate::social::listen::seal::{self, Channel};
use crate::sync::keycopy::Qr;

const WORD: &str = "share";
/// Links open at once.
const MAX_LINKS: usize = 8;

/// What links need from the app.
pub struct LinkDeps {
    pub relays: Arc<dyn RelayFactory>,
    /// The relay setting (listen along's); empty means `default_relay`.
    pub relay: Arc<RwLock<String>>,
    pub default_relay: String,
    /// How long a friend waits for the host's answer.
    pub answer_timeout: Duration,
    pub retry: Duration,
}

/// Friend → host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Ask {
    /// Names this attempt (sent again unchanged if the answer is lost).
    n: String,
    /// Their Plex username.
    u: String,
    /// The name they go by in Platter.
    name: String,
    /// Unix ms; older than a few minutes is dropped.
    x: u64,
}

/// Host → friend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Answer {
    n: String,
    /// "invited", "already", "used", "expired", "failed".
    s: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    msg: Option<String>,
    host: String,
}

/// `sharing_link`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkView {
    pub link: String,
    pub qr: Option<Qr>,
    /// Unix ms.
    pub expires_at: u64,
}

/// What opening a link did (the friend's side).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum LinkResult {
    /// The host shared; the invite shows in Platter shortly.
    Invited { host: String },
    AlreadyShared { host: String },
    Failed { message: String },
}

struct Open {
    up: Channel,
    down: Channel,
    expires_ms: u64,
    opts: ShareOptions,
    /// The attempt that used it and the answer (sent again if asked).
    used: Option<(String, Answer)>,
}

pub(super) struct Links {
    deps: LinkDeps,
    open: Mutex<Vec<Open>>,
    topics: watch::Sender<Vec<String>>,
    running: Mutex<bool>,
}

fn channels(t: &Token) -> (Channel, Channel) {
    let seed = t.seed();
    (Channel::new(&seed, "share up"), Channel::new(&seed, "share down"))
}

fn relay_words(e: RelayError) -> AppError {
    match e {
        RelayError::Limited => AppError::Network("The relay is busy. Try again in a while.".into()),
        _ => AppError::Network("Couldn't reach the relay.".into()),
    }
}

impl Links {
    pub(super) fn new(deps: LinkDeps) -> Self {
        Self { deps, open: Mutex::default(), topics: watch::channel(Vec::new()).0, running: Mutex::new(false) }
    }

    fn base(&self) -> String {
        invite::relay_url(&self.deps.relay.read()).unwrap_or_else(|| self.deps.default_relay.clone())
    }

    fn refresh(&self) {
        let now = now_ms();
        let t: Vec<String> = {
            let mut open = self.open.lock();
            open.retain(|o| o.expires_ms > now);
            open.iter().map(|o| o.up.topic.clone()).collect()
        };
        self.topics.send_if_modified(|old| std::mem::replace(old, t.clone()) != t);
    }
}

impl Sharing {
    fn links(&self) -> &Links {
        &self.inner.links
    }

    /// A new share link for these options (host). Good for one person,
    /// for `hours`.
    pub async fn make_link(&self, opts: ShareOptions, hours: u32) -> AppResult<LinkView> {
        self.owned_server().await?;
        if opts.libraries.is_empty() {
            return Err(AppError::Invalid("Pick a music library to share.".into()));
        }
        let l = self.links();
        let base = l.base();
        let expires_ms = now_ms() + u64::from(hours.clamp(1, 24 * 7)) * 3_600_000;
        let token = Token {
            session: seal::random16(),
            secret: seal::random16(),
            expires: (expires_ms / 1000) as u32,
            relay: (base != l.deps.default_relay).then_some(base.clone()),
        };
        let (up, down) = channels(&token);
        {
            let mut open = l.open.lock();
            while open.len() >= MAX_LINKS {
                open.remove(0);
            }
            open.push(Open { up, down, expires_ms, opts, used: None });
        }
        l.refresh();
        let start = !std::mem::replace(&mut *l.running.lock(), true);
        if start {
            let relay = l.deps.relays.relay(&base);
            tokio::spawn(self.clone().pump(relay));
        }
        let link = token.link_as(WORD);
        tracing::info!("made a share link");
        Ok(LinkView { qr: crate::social::listen::qr_for(&link), link, expires_at: expires_ms })
    }

    /// Listen on the open links' topics while there are any.
    async fn pump(self, relay: Arc<dyn Relay>) {
        let l = self.links();
        let mut topics = l.topics.subscribe();
        let mut wait = l.deps.retry;
        loop {
            l.refresh();
            let list = topics.borrow_and_update().clone();
            if list.is_empty() {
                *l.running.lock() = false;
                return;
            }
            match relay.subscribe(&list, None).await {
                Err(_) => {
                    tokio::time::sleep(wait).await;
                    wait = (wait * 2).min(Duration::from_secs(60));
                }
                Ok(mut rx) => loop {
                    tokio::select! {
                        ev = rx.recv() => match ev {
                            Some(RelayEvent::Open) => wait = l.deps.retry,
                            Some(RelayEvent::Message { topic, body, .. }) => self.on_ask(&relay, &topic, &body).await,
                            None => {
                                tokio::time::sleep(wait).await;
                                break;
                            }
                        },
                        r = topics.changed() => {
                            if r.is_err() {
                                return;
                            }
                            break;
                        }
                        _ = tokio::time::sleep(Duration::from_secs(60)) => {
                            // Expired links drop off.
                            l.refresh();
                        }
                    }
                },
            }
        }
    }

    async fn on_ask(&self, relay: &Arc<dyn Relay>, topic: &str, body: &str) {
        let l = self.links();
        let now = now_ms();
        let host = self.inner.host.username().await.unwrap_or_default();
        enum Do {
            Reply(Channel, Answer),
            Share(Channel, String, ShareOptions, String, String),
        }
        let todo = {
            let mut open = l.open.lock();
            let Some(o) = open.iter_mut().find(|o| o.up.topic == topic) else { return };
            let Ok(ask) = seal::open_value::<Ask>(&o.up, body) else { return };
            if ask.n.len() > 64 || ask.x + seal::TTL_MS < now || ask.x > now + seal::TTL_MS {
                return;
            }
            let u = ask.u.trim().chars().filter(|c| !c.is_control()).take(200).collect::<String>();
            match &o.used {
                Some((n, a)) if *n == ask.n => Do::Reply(o.down.clone(), a.clone()),
                Some(_) => Do::Reply(o.down.clone(), Answer { n: ask.n, s: "used".into(), msg: None, host: host.clone() }),
                None if o.expires_ms <= now => Do::Reply(o.down.clone(), Answer { n: ask.n, s: "expired".into(), msg: None, host: host.clone() }),
                None if u.is_empty() => return,
                None => {
                    o.used = Some((ask.n.clone(), Answer { n: ask.n.clone(), s: "failed".into(), msg: Some("Still sharing. Try again in a moment.".into()), host: host.clone() }));
                    let name: String = ask.name.chars().filter(|c| !c.is_control()).take(40).collect();
                    Do::Share(o.down.clone(), ask.n, o.opts.clone(), u, name)
                }
            }
        };
        let (down, answer) = match todo {
            Do::Reply(down, a) => (down, a),
            Do::Share(down, n, opts, user, name) => {
                let st = self.share(&user, &opts).await;
                let (s, msg) = match &st {
                    Ok(ShareStatus::Invited { .. }) => ("invited", None),
                    Ok(ShareStatus::AlreadyShared) => ("already", None),
                    Ok(other) => ("failed", Some(Self::status_text(other))),
                    Err(e) => ("failed", Some(e.to_string().split_once(": ").map(|x| x.1.to_string()).unwrap_or_default())),
                };
                let who = if name.trim().is_empty() { "Someone".to_string() } else { name };
                let words = match s {
                    "invited" => format!("{who} opened your share link. Plex sent them an invite."),
                    "already" => format!("{who} opened your share link. They already have access."),
                    _ => format!("{who} opened your share link, but sharing didn't work: {}", msg.clone().unwrap_or_default()),
                };
                self.notice(words);
                let a = Answer { n: n.clone(), s: s.into(), msg, host };
                // Used for good, unless it failed (then the same person may try again).
                if let Some(o) = l.open.lock().iter_mut().find(|o| o.down.topic == down.topic) {
                    o.used = if a.s == "failed" { None } else { Some((n, a.clone())) };
                }
                (down, a)
            }
        };
        match seal::seal_value(&down, &answer) {
            Ok(text) => {
                if let Err(e) = relay.publish(&down.topic, &text).await {
                    tracing::debug!(err = ?e, "share link answer not sent");
                }
            }
            Err(e) => tracing::debug!(kind = e.kind(), "share link answer not sealed"),
        }
    }

    /// Open a share link someone gave you (the friend's side): ask for their
    /// music, wait for the answer, then look for the invite.
    pub async fn open_link(&self, link: &str, my_name: &str) -> AppResult<LinkResult> {
        let token = invite::parse_as(link, WORD).ok_or_else(|| AppError::Invalid("That isn't a share link.".into()))?;
        if u64::from(token.expires) * 1000 <= now_ms() {
            return Ok(LinkResult::Failed { message: "That link has expired. Ask for a new one.".into() });
        }
        let user = self.inner.host.username().await.ok_or_else(|| AppError::Auth("Sign in to Plex first.".into()))?;
        let l = self.links();
        let base = token.relay.clone().unwrap_or_else(|| l.deps.default_relay.clone());
        let relay = l.deps.relays.relay(&base);
        let (up, down) = channels(&token);
        let mut rx = relay.subscribe(std::slice::from_ref(&down.topic), None).await.map_err(relay_words)?;
        let deadline = Instant::now() + l.deps.answer_timeout;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(RelayEvent::Open)) => break,
                Ok(Some(_)) => continue,
                _ => return Err(relay_words(RelayError::Unreachable)),
            }
        }
        let n = hex(&seal::random16()[..8]);
        let ask = Ask { n: n.clone(), u: user, name: my_name.chars().take(40).collect(), x: now_ms() };
        let text = seal::seal_value(&up, &ask)?;
        relay.publish(&up.topic, &text).await.map_err(relay_words)?;
        let answer = loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(RelayEvent::Message { topic, body, .. })) if topic == down.topic => {
                    if let Ok(a) = seal::open_value::<Answer>(&down, &body)
                        && a.n == n
                    {
                        break a;
                    }
                }
                Ok(Some(_)) => {}
                Ok(None) => return Err(relay_words(RelayError::Unreachable)),
                Err(_) => {
                    return Ok(LinkResult::Failed { message: "No answer. They need Platter open for the link to work.".into() });
                }
            }
        };
        let host = if answer.host.trim().is_empty() { "They".to_string() } else { answer.host.chars().take(60).collect() };
        let out = match answer.s.as_str() {
            "invited" => LinkResult::Invited { host: host.clone() },
            "already" => LinkResult::AlreadyShared { host: host.clone() },
            "used" => LinkResult::Failed { message: "That link was already used. Ask for a new one.".into() },
            "expired" => LinkResult::Failed { message: "That link has expired. Ask for a new one.".into() },
            _ => LinkResult::Failed { message: answer.msg.unwrap_or_else(|| "Sharing didn't work on their side.".into()) },
        };
        if matches!(out, LinkResult::Invited { .. } | LinkResult::AlreadyShared { .. }) {
            let me = self.clone();
            tokio::spawn(async move { crate::social::listen::ServerOffer::shared(&me, &host).await });
        }
        Ok(out)
    }

    /// Open share links (host), for tests and the UI's count.
    pub fn open_links(&self) -> usize {
        self.links().refresh();
        self.links().open.lock().len()
    }
}
