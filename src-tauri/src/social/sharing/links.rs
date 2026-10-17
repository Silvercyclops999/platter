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
    /// The request waits for the host's yes; an invite follows if they say it.
    Asked { host: String },
    AlreadyShared { host: String },
    Failed { message: String },
}

struct Open {
    up: Channel,
    down: Channel,
    expires_ms: u64,
    opts: ShareOptions,
    /// The host said yes to someone and sharing worked: done.
    used: bool,
    /// Requests waiting for the host's yes.
    asks: Vec<LinkAsk>,
    /// Shares that didn't work; the link closes after [`MAX_FAILURES`].
    failures: u32,
}

/// Requests one link holds at once.
const MAX_ASKS: usize = 3;
/// Shares from one link that may fail before it closes.
const MAX_FAILURES: u32 = 3;

/// The requests waiting for a yes changed (a list of [`LinkAsk`]).
pub const EVENT_LINK_ASKS: &str = "sharing://link-asks";

/// Someone who opened a share link and asks for the host's music.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkAsk {
    pub id: String,
    #[serde(skip)]
    n: String,
    /// Their Plex username.
    pub user: String,
    /// The name they go by in Platter.
    pub name: String,
    /// Unix ms.
    pub at: u64,
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
            open.push(Open { up, down, expires_ms, opts, used: false, asks: Vec::new(), failures: 0 });
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

    /// Someone opened a link: their request waits for the host's yes
    /// ([`Self::approve_link_ask`]). The answer is always "asked", whoever
    /// they say they are, so a link can't be used to try out usernames.
    async fn on_ask(&self, relay: &Arc<dyn Relay>, topic: &str, body: &str) {
        let l = self.links();
        let now = now_ms();
        let host = self.inner.host.username().await.unwrap_or_default();
        let mut asked: Option<LinkAsk> = None;
        let (down, answer) = {
            let mut open = l.open.lock();
            let Some(o) = open.iter_mut().find(|o| o.up.topic == topic) else { return };
            let Ok(ask) = seal::open_value::<Ask>(&o.up, body) else { return };
            if ask.n.len() > 64 || ask.x + seal::TTL_MS < now || ask.x > now + seal::TTL_MS {
                return;
            }
            let u = ask.u.trim().chars().filter(|c| !c.is_control()).take(200).collect::<String>();
            let reply = |s: &str| Answer { n: ask.n.clone(), s: s.into(), msg: None, host: host.clone() };
            let a = if o.used {
                reply("used")
            } else if o.expires_ms <= now {
                reply("expired")
            } else if u.is_empty() {
                return;
            } else {
                if !o.asks.iter().any(|a| a.n == ask.n) && o.asks.len() < MAX_ASKS {
                    let name: String = ask.name.chars().filter(|c| !c.is_control()).take(40).collect();
                    let a = LinkAsk { id: hex(&seal::random16()[..8]), n: ask.n.clone(), user: u, name: if name.trim().is_empty() { "Someone".into() } else { name }, at: now };
                    o.asks.push(a.clone());
                    asked = Some(a);
                }
                reply("asked")
            };
            (o.down.clone(), a)
        };
        if let Some(a) = asked {
            self.notice(format!("{} ({}) opened your share link and asks for your music. Say yes in Settings, Sharing.", a.name, a.user));
            self.inner.events.emit(EVENT_LINK_ASKS, serde_json::to_value(self.link_asks()).unwrap_or_default());
        }
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
        if !invite::link_relay_ok(token.relay.as_deref(), &self.links().deps.relay.read()) {
            return Ok(LinkResult::Failed { message: invite::UNSAFE_RELAY.into() });
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
            "asked" => LinkResult::Asked { host: host.clone() },
            "invited" => LinkResult::Invited { host: host.clone() },
            "already" => LinkResult::AlreadyShared { host: host.clone() },
            "used" => LinkResult::Failed { message: "That link was already used. Ask for a new one.".into() },
            "expired" => LinkResult::Failed { message: "That link has expired. Ask for a new one.".into() },
            _ => LinkResult::Failed { message: answer.msg.unwrap_or_else(|| "Sharing didn't work on their side.".into()) },
        };
        if matches!(out, LinkResult::Asked { .. } | LinkResult::Invited { .. } | LinkResult::AlreadyShared { .. }) {
            let me = self.clone();
            tokio::spawn(async move { crate::social::listen::ServerOffer::shared(&me, &host).await });
        }
        Ok(out)
    }

    /// Requests waiting for the host's yes, oldest first.
    pub fn link_asks(&self) -> Vec<LinkAsk> {
        self.links().refresh();
        let mut all: Vec<LinkAsk> = self.links().open.lock().iter().flat_map(|o| o.asks.clone()).collect();
        all.sort_by_key(|a| a.at);
        all
    }

    fn take_ask(&self, id: &str) -> Option<(LinkAsk, ShareOptions, String)> {
        let mut open = self.links().open.lock();
        let o = open.iter_mut().find(|o| o.asks.iter().any(|a| a.id == id))?;
        let i = o.asks.iter().position(|a| a.id == id)?;
        let a = o.asks.remove(i);
        (!o.used).then(|| (a, o.opts.clone(), o.down.topic.clone()))
    }

    /// The host said no (or let it be): the request goes away.
    pub fn deny_link_ask(&self, id: &str) {
        let _ = self.take_ask(id);
        self.inner.events.emit(EVENT_LINK_ASKS, serde_json::to_value(self.link_asks()).unwrap_or_default());
    }

    /// The host said yes: share with them, as the link was made. Returns
    /// what happened, in words.
    pub async fn approve_link_ask(&self, id: &str) -> AppResult<String> {
        let (a, opts, topic) = self.take_ask(id).ok_or_else(|| AppError::NotFound("That request isn't waiting any more.".into()))?;
        let st = self.share(&a.user, &opts).await;
        let ok = matches!(st, Ok(ShareStatus::Invited { .. } | ShareStatus::AlreadyShared));
        {
            let mut open = self.links().open.lock();
            if let Some(i) = open.iter().position(|o| o.down.topic == topic) {
                if ok {
                    open[i].used = true;
                    open[i].asks.clear();
                } else {
                    open[i].failures += 1;
                    if open[i].failures >= MAX_FAILURES {
                        open.remove(i);
                    }
                }
            }
        }
        self.links().refresh();
        self.inner.events.emit(EVENT_LINK_ASKS, serde_json::to_value(self.link_asks()).unwrap_or_default());
        Ok(match st {
            Ok(ShareStatus::Invited { .. }) => format!("Shared with {}. Plex sent them an invite.", a.name),
            Ok(ShareStatus::AlreadyShared) => format!("{} already has access.", a.name),
            Ok(other) => format!("Sharing with {} didn't work: {}", a.name, Self::status_text(&other)),
            Err(e) => format!("Sharing with {} didn't work: {}", a.name, e.message()),
        })
    }

    /// Open share links (host), for tests and the UI's count.
    pub fn open_links(&self) -> usize {
        self.links().refresh();
        self.links().open.lock().len()
    }
}
