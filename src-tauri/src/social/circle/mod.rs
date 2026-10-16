//! Collaborative playlists and Blends, between people who each play from
//! the Plex servers they already reach. Nothing but metadata leaves this
//! computer, and only through the listen along relay (an ntfy server,
//! [`crate::social::listen::relay`]), sealed end to end. There's no Platter
//! server.
//!
//! - [`link`]: a sealed link between two people with an outbox, so a change
//!   made while the other side is offline is sent again until it's
//!   acknowledged (the relay holds messages for hours only).
//! - Invites are listen along's one-use, expiring tokens
//!   ([`crate::social::listen::invite`]) under their own link words
//!   (`platter://collab/...`, `platter://blend/...`). The first person who
//!   answers agrees a key with whoever made the invite (SPAKE2 over the
//!   invite's secret) and gets their link's secret sealed with it. People on
//!   the same server or Plex Home can be sent an invite directly: it goes to
//!   their doorbell, a channel only people who reach that server can work
//!   out, and still has to be answered like any invite.
//! - [`collab`]: a shared playlist is a normal Plex playlist. Members' edits
//!   go to the owner, whose Platter applies them to Plex and sends everyone
//!   the change. Members see the playlist from the songs they can play here.
//! - [`blend`]: everyone in a Blend shares one channel and sends a small
//!   taste summary on it; each person's Platter makes the playlist from the
//!   summaries and the songs it can play.

pub mod blend;
pub mod collab;
pub mod commands;
pub mod library;
pub mod link;
pub mod wire;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, oneshot, watch};

use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::plex::state::EventSink;
use crate::remote::peer::auth::{hex, hmac_sha256};
use crate::social::listen::invite::{self, Token};
use crate::social::listen::relay::{Relay, RelayEvent, RelayFactory, RelayHealth};
use crate::social::listen::seal::{self, Channel, PakeGuest};
use blend::Made;
use link::{End, Link, Packet, Pieces};
use wire::{Delta, EditOp, Entry, Ident, Kind, Member, Msg, Person, Role, Snapshot, Taste, Welcome, Why};

pub const EVENT: &str = "circle://changed";
/// Something worth a toast (an edit that couldn't be applied, a playlist
/// that was deleted, an invite that arrived).
pub const EVENT_NOTICE: &str = "circle://notice";
/// Open invites one playlist or Blend keeps.
pub const MAX_INVITES: usize = 16;
pub const NAME_MAX: usize = 40;
/// An expired invite is still answered ("expired") for this long, ms.
const EXPIRED_KEPT: u64 = 86_400_000;
const WORD_PLAYLIST: &str = "collab";
const WORD_BLEND: &str = "blend";
/// Route ids the UI gets for playlists shared with you and for Blends.
pub const SHARED_PREFIX: &str = "shared:";
pub const BLEND_PREFIX: &str = "blend:";

/// The settings this reads (synced).
#[derive(Clone)]
pub struct CircleSettings {
    /// "Collaborative playlists" (on; nothing happens without an invite).
    pub playlists: Arc<AtomicBool>,
    /// "Blends" (on; nothing happens without an invite).
    pub blends: Arc<AtomicBool>,
    /// "Include unreleased songs in what I share" (off).
    pub share_unreleased: Arc<AtomicBool>,
    /// A private session: no taste is made or sent.
    pub private_session: Arc<AtomicBool>,
    /// Listen along's relay and invite lifetime.
    pub relay: Arc<RwLock<String>>,
    pub invite_hours: Arc<AtomicU32>,
}

impl Default for CircleSettings {
    fn default() -> Self {
        Self {
            playlists: Arc::new(AtomicBool::new(true)),
            blends: Arc::new(AtomicBool::new(true)),
            share_unreleased: Arc::new(AtomicBool::new(false)),
            private_session: Arc::new(AtomicBool::new(false)),
            relay: Arc::default(),
            invite_hours: Arc::new(AtomicU32::new(24)),
        }
    }
}

/// Timing (shorter in tests).
#[derive(Debug, Clone, Copy)]
pub struct CircleConfig {
    /// Look at owned playlists, outboxes and Blends this often.
    pub tick: Duration,
    /// First wait before sending an outbox again, and the longest.
    pub resend_first: Duration,
    pub resend_max: Duration,
    /// How long joining waits for an answer before it says it's waiting.
    pub answer_timeout: Duration,
    /// Reconnecting to the relay.
    pub retry: Duration,
    pub retry_max: Duration,
    /// A taste summary is sent again this often.
    pub taste_every: Duration,
}

impl Default for CircleConfig {
    fn default() -> Self {
        Self {
            tick: Duration::from_secs(15),
            resend_first: Duration::from_secs(60),
            resend_max: Duration::from_secs(30 * 60),
            answer_timeout: Duration::from_secs(20),
            retry: Duration::from_secs(2),
            retry_max: Duration::from_secs(60),
            taste_every: Duration::from_secs(20 * 3600),
        }
    }
}

// ---- the outside world --------------------------------------------------------

/// An owned playlist as Plex (the local cache) has it now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rows {
    pub title: String,
    pub summary: Option<String>,
    /// (row id, song id), in order.
    pub rows: Vec<(String, String)>,
}

/// Your own Plex playlists (the app's playlist editing; a fake in tests).
#[async_trait]
pub trait OwnerPlaylists: Send + Sync {
    /// None: it was deleted.
    async fn rows(&self, id: &str) -> AppResult<Option<Rows>>;
    async fn add(&self, id: &str, track_ids: Vec<String>) -> AppResult<()>;
    async fn remove(&self, id: &str, item_ids: Vec<String>) -> AppResult<()>;
}

/// The library, as this needs it.
#[async_trait]
pub trait CircleLibrary: Send + Sync {
    /// Who these songs are (ids not in the library are left out).
    async fn describe(&self, ids: Vec<String>) -> HashMap<String, Ident>;
    /// The copy here of each song, same order.
    async fn resolve(&self, songs: Vec<Ident>) -> Vec<Option<String>>;
    /// This computer's taste summary now.
    async fn taste(&self, now_s: i64, unreleased: bool) -> Taste;
    /// Make a Blend from these members' tastes ([`blend::generate`]).
    async fn blend(&self, members: Vec<(String, Taste)>, hidden: HashSet<String>, unreleased: bool, seed: u64, me: String) -> Made;
    async fn tracks(&self, ids: Vec<String>) -> Vec<Track>;
}

/// Someone on the same server or Plex Home.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    /// Their doorbell name ([`doorbell`]).
    pub key: String,
    pub name: String,
    pub avatar: Option<String>,
    /// In your Plex Home.
    pub home: bool,
}

#[async_trait]
pub trait People: Send + Sync {
    /// This user's own doorbell names.
    async fn me(&self) -> Vec<String>;
    /// Other people on this server or Home.
    async fn others(&self) -> Vec<Account>;
    /// The connected server's machine id (it never leaves this computer).
    async fn server(&self) -> Option<String>;
}

/// Where the state is kept between runs.
pub trait CircleStore: Send + Sync {
    fn load(&self) -> Option<String>;
    fn save(&self, json: &str);
}

/// Kept in memory (tests).
#[derive(Default)]
pub struct MemStore(pub Mutex<Option<String>>);

impl CircleStore for MemStore {
    fn load(&self) -> Option<String> {
        self.0.lock().clone()
    }
    fn save(&self, json: &str) {
        *self.0.lock() = Some(json.to_string());
    }
}

/// Sees every packet before it's sealed (tests).
pub type Tap = Arc<dyn Fn(&str) + Send + Sync>;

pub struct CircleDeps {
    pub relays: Arc<dyn RelayFactory>,
    pub events: Arc<dyn EventSink>,
    pub store: Arc<dyn CircleStore>,
    pub library: Arc<dyn CircleLibrary>,
    pub playlists: Arc<dyn OwnerPlaylists>,
    pub people: Arc<dyn People>,
    /// The name and picture others see.
    pub name: Arc<dyn Fn() -> String + Send + Sync>,
    pub avatar: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    pub settings: CircleSettings,
    pub default_relay: String,
    pub cfg: CircleConfig,
    /// Unix ms now.
    pub clock: Arc<dyn Fn() -> u64 + Send + Sync>,
    pub tap: Option<Tap>,
}

// ---- what's kept ----------------------------------------------------------------

/// An invite someone may still answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Invite {
    /// The token's seed (base64).
    seed: String,
    /// Unix ms.
    expires: u64,
    link: String,
    #[serde(default)]
    role: Option<Role>,
    /// The attempt that got in, and the answer it got (sent again if asked).
    #[serde(default)]
    redeemed: Option<(String, Msg)>,
}

fn invite_channels(seed: &[u8]) -> (Channel, Channel) {
    (Channel::new(seed, "circle invite up"), Channel::new(seed, "circle invite down"))
}

impl Invite {
    fn channels(&self) -> (Channel, Channel) {
        invite_channels(&seal::unb64(&self.seed).unwrap_or_default())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct OwnedMember {
    p: Person,
    role: Role,
    link: Link,
    /// Taken off: told so, then forgotten once they've heard.
    #[serde(default)]
    out: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Owned {
    playlist: String,
    members: Vec<OwnedMember>,
    invites: Vec<Invite>,
    /// Songs a collaborator asked for that the owner's next read should
    /// find (song id, person id).
    #[serde(default)]
    claims: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct PendingEdit {
    n: String,
    op: EditOp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Joined {
    me: String,
    role: Role,
    link: Link,
    #[serde(default)]
    pending: Vec<PendingEdit>,
    /// Left: forgotten once the owner has heard.
    #[serde(default)]
    leaving: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Collab {
    g: String,
    title: String,
    #[serde(default)]
    summary: Option<String>,
    owner: Person,
    public: bool,
    v: u64,
    songs: Vec<Entry>,
    ppl: Vec<Member>,
    /// Deleted by the owner, or you were taken off.
    #[serde(default)]
    gone: bool,
    #[serde(default = "yes")]
    in_library: bool,
    #[serde(default)]
    mine: Option<Owned>,
    #[serde(default)]
    joined: Option<Joined>,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BlendMember {
    p: Person,
    #[serde(default)]
    taste: Option<Taste>,
    #[serde(default)]
    left: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct BlendState {
    g: String,
    me: Person,
    /// The secret everyone in it shares (base64).
    group: String,
    /// Everyone else.
    members: Vec<BlendMember>,
    #[serde(default)]
    invites: Vec<Invite>,
    /// Songs hidden in this Blend (song ids here); they don't come back.
    #[serde(default)]
    hidden: Vec<String>,
    #[serde(default)]
    made: Made,
    /// The day it was made for, and who was in.
    #[serde(default)]
    day: i64,
    #[serde(default)]
    made_with: Vec<String>,
    /// The last counter seen from each member.
    #[serde(default)]
    guard: HashMap<String, u64>,
    #[serde(default = "one")]
    next: u64,
    /// When this computer's taste last went out (unix ms; 0: send now).
    #[serde(default)]
    sent_taste: u64,
}

fn one() -> u64 {
    1
}

impl BlendState {
    fn channel(&self) -> Channel {
        Channel::new(&seal::key32(&self.group).unwrap_or([0; 32]), "blend")
    }
    fn active(&self) -> impl Iterator<Item = &BlendMember> {
        self.members.iter().filter(|m| !m.left)
    }
    fn names(&self) -> Vec<String> {
        std::iter::once(self.me.name.clone()).chain(self.active().map(|m| m.p.name.clone())).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Incoming {
    id: String,
    from: String,
    kind: Kind,
    title: String,
    link: String,
    /// Unix ms.
    at: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    collabs: Vec<Collab>,
    #[serde(default)]
    blends: Vec<BlendState>,
    #[serde(default)]
    incoming: Vec<Incoming>,
    /// The relay's last message id seen.
    #[serde(default)]
    since: Option<String>,
}

/// An invite being answered (kept in memory: the key agreement's state
/// can't be saved).
struct Joining {
    kind: Kind,
    n: String,
    msg_a: Vec<u8>,
    pake: Option<PakeGuest>,
    up: Channel,
    down: Channel,
    expires: u64,
    sent: u64,
    done: Option<oneshot::Sender<AppResult<String>>>,
}

struct St {
    saved: Saved,
    joining: Vec<Joining>,
    pieces: Pieces,
    doorbells: Vec<Channel>,
    /// Bound to this server: what direct invites go out on.
    server: Option<String>,
}

// ---- what the UI sees ---------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonView {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub role: Option<Role>,
    pub you: bool,
    /// Songs they added.
    pub added: u32,
}

/// A shared playlist (yours or someone else's).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedSummary {
    /// The page to open: your Plex playlist's id, or `shared:<group>`.
    pub id: String,
    pub group: String,
    pub title: String,
    pub owner: PersonView,
    pub role: Role,
    /// The owner first, then everyone it's shared with.
    pub people: Vec<PersonView>,
    pub public: bool,
    pub gone: bool,
    pub in_library: bool,
    /// Your edits the owner hasn't confirmed yet.
    pub waiting: u32,
    /// Sent again a few times without an answer.
    pub owner_unreachable: bool,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlendSummary {
    pub id: String,
    pub title: String,
    pub description: String,
    /// You first.
    pub people: Vec<PersonView>,
    pub track_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IncomingView {
    pub id: String,
    pub from: String,
    pub kind: Kind,
    pub title: String,
    pub at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CircleView {
    pub relay: RelayHealth,
    pub playlists: Vec<SharedSummary>,
    pub blends: Vec<BlendSummary>,
    pub incoming: Vec<IncomingView>,
    /// Invites waiting for their owner's answer.
    pub joining: u32,
}

/// One row of a shared playlist page.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedRow {
    pub key: String,
    /// The copy here; None: "Not in your library".
    pub track: Option<Track>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub duration_ms: u64,
    pub release_group_id: Option<String>,
    pub by: Option<PersonView>,
    pub added_at: i64,
    /// Your edit, not confirmed by the owner yet.
    pub pending: bool,
    /// You may remove it.
    pub removable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedPage {
    pub summary: SharedSummary,
    pub description: Option<String>,
    pub duration_ms: u64,
    pub rows: Vec<SharedRow>,
}

/// The owner's view of who added what on their own playlist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedInfo {
    pub summary: SharedSummary,
    /// Row id → who added it (rows the owner added are left out).
    pub added_by: HashMap<String, PersonView>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlendRowView {
    pub track: Track,
    pub by: PersonView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingView {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub release_group_id: Option<String>,
    pub by: PersonView,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlendPage {
    pub summary: BlendSummary,
    pub rows: Vec<BlendRowView>,
    pub missing: Vec<MissingView>,
    pub duration_ms: u64,
}

/// What joining did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum JoinResult {
    /// In: the page to open.
    Joined { id: String, kind: Kind },
    /// Sent; the owner answers when they're next online.
    Waiting { kind: Kind },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteLink {
    pub link: String,
    /// Unix ms.
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlendCreated {
    pub id: String,
    pub invite: InviteLink,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Notice {
    message: String,
}

// ---- helpers ---------------------------------------------------------------------

fn rid() -> String {
    hex(&seal::random16()[..8])
}

fn clean_name(s: &str) -> String {
    let n: String = s.chars().filter(|c| !c.is_control()).take(NAME_MAX).collect();
    n.trim().to_string()
}

/// A picture others may see: a Plex `/ext/` art path, nothing with a token.
fn clean_avatar(a: Option<String>) -> Option<String> {
    a.filter(|a| a.starts_with("/ext/") && a.len() <= 300 && !a.to_ascii_lowercase().contains("token") && !a.chars().any(char::is_whitespace))
}

fn clean_person(p: Person) -> Person {
    let name = clean_name(&p.name);
    Person { id: p.id.chars().take(32).collect(), name: if name.is_empty() { "Someone".into() } else { name }, av: clean_avatar(p.av) }
}

fn welcome_aad(n: &str) -> String {
    format!("platter-circle-welcome\n{n}")
}

/// A shared playlist or Blend invite link.
pub fn is_invite(raw: &str) -> bool {
    invite::parse_as(raw, WORD_PLAYLIST).is_some() || invite::parse_as(raw, WORD_BLEND).is_some()
}

/// Where direct invites to someone on a server go: a channel from the
/// server's machine id (which only people who reach it know) and their
/// name there. The relay sees a random topic.
pub fn doorbell(machine_id: &str, key: &str) -> Channel {
    let secret = hmac_sha256(machine_id.as_bytes(), b"platter circle doorbell");
    Channel::new(&secret, &format!("doorbell {}", key.to_lowercase()))
}

fn role_word(r: Role) -> &'static str {
    match r {
        Role::Owner => "owner",
        Role::Collaborator => "collaborator",
        Role::Listener => "listener",
        Role::Follower => "follower",
    }
}

fn relay_err() -> AppError {
    AppError::Network("Couldn't reach the relay. Changes wait here and go out when it's back.".into())
}

fn snapshot(c: &Collab, owner: &Person, you: Role) -> Snapshot {
    Snapshot {
        v: c.v,
        title: c.title.clone(),
        sum: c.summary.clone(),
        public: c.public,
        owner: owner.clone(),
        ppl: c.ppl.clone(),
        you,
        songs: c.songs.clone(),
    }
}

/// The people list from the owner's members.
fn member_list(o: &Owned) -> Vec<Member> {
    o.members.iter().filter(|m| !m.out && m.role != Role::Follower).map(|m| Member { p: m.p.clone(), r: m.role }).collect()
}

// ---- the service -------------------------------------------------------------------

struct Inner {
    deps: CircleDeps,
    st: tokio::sync::Mutex<St>,
    shown: Mutex<CircleView>,
    health: Mutex<RelayHealth>,
    relay: Arc<dyn Relay>,
    relay_base: String,
    topics: watch::Sender<Vec<String>>,
    poke: Notify,
    started: AtomicBool,
}

/// Collaborative playlists and Blends on this computer. Cheap to clone.
#[derive(Clone)]
pub struct Circle {
    inner: Arc<Inner>,
}

type Outgoing = Vec<(Channel, Packet)>;

impl Circle {
    pub fn new(deps: CircleDeps) -> Self {
        let saved: Saved = deps.store.load().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let base = invite::relay_url(&deps.settings.relay.read()).unwrap_or_else(|| deps.default_relay.clone());
        let relay = deps.relays.relay(&base);
        let inner = Inner {
            st: tokio::sync::Mutex::new(St { saved, joining: Vec::new(), pieces: Pieces::default(), doorbells: Vec::new(), server: None }),
            shown: Mutex::new(CircleView { relay: RelayHealth::Ok, playlists: Vec::new(), blends: Vec::new(), incoming: Vec::new(), joining: 0 }),
            health: Mutex::new(RelayHealth::Ok),
            relay,
            relay_base: base,
            topics: watch::channel(Vec::new()).0,
            poke: Notify::new(),
            started: AtomicBool::new(false),
            deps,
        };
        Self { inner: Arc::new(inner) }
    }

    fn now(&self) -> u64 {
        (self.inner.deps.clock)()
    }

    fn now_s(&self) -> i64 {
        (self.now() / 1000) as i64
    }

    fn settings(&self) -> &CircleSettings {
        &self.inner.deps.settings
    }

    fn unreleased_ok(&self) -> bool {
        self.settings().share_unreleased.load(Ordering::Relaxed)
    }

    fn me_person(&self, id: &str) -> Person {
        clean_person(Person { id: id.into(), name: (self.inner.deps.name)(), av: (self.inner.deps.avatar)() })
    }

    /// Start listening and the background work (once).
    pub async fn start(&self) {
        if self.inner.started.swap(true, Ordering::SeqCst) {
            return;
        }
        self.refresh_doorbells().await;
        {
            let st = self.inner.st.lock().await;
            self.after(&st);
        }
        tokio::spawn(self.clone().pump());
        tokio::spawn(self.clone().ticker());
    }

    /// Look at things now (after the playlist was edited here).
    pub fn kick(&self) {
        self.inner.poke.notify_one();
    }

    pub fn view(&self) -> CircleView {
        self.inner.shown.lock().clone()
    }

    fn notice(&self, message: String) {
        self.inner.deps.events.emit(EVENT_NOTICE, serde_json::to_value(Notice { message }).unwrap_or_default());
    }

    /// Save, update what the UI sees, and listen on the right topics.
    fn after(&self, st: &St) {
        if let Ok(json) = serde_json::to_string(&st.saved) {
            self.inner.deps.store.save(&json);
        }
        let v = self.build_view(st);
        let changed = {
            let mut shown = self.inner.shown.lock();
            let c = *shown != v;
            *shown = v.clone();
            c
        };
        if changed {
            self.inner.deps.events.emit(EVENT, serde_json::to_value(&v).unwrap_or_default());
        }
        let topics = self.topic_list(st);
        self.inner.topics.send_if_modified(|t| std::mem::replace(t, topics.clone()) != topics);
    }

    fn topic_list(&self, st: &St) -> Vec<String> {
        let now = self.now();
        let mut t: Vec<String> = Vec::new();
        for c in &st.saved.collabs {
            if let Some(o) = &c.mine {
                t.extend(o.invites.iter().filter(|i| i.expires + EXPIRED_KEPT > now).map(|i| i.channels().0.topic));
                t.extend(o.members.iter().map(|m| m.link.channels(End::Owner).1.topic));
            }
            if let Some(j) = &c.joined {
                t.push(j.link.channels(End::Member).1.topic);
            }
        }
        for b in &st.saved.blends {
            t.push(b.channel().topic);
            t.extend(b.invites.iter().filter(|i| i.expires + EXPIRED_KEPT > now).map(|i| i.channels().0.topic));
        }
        t.extend(st.joining.iter().map(|j| j.down.topic.clone()));
        t.extend(st.doorbells.iter().map(|d| d.topic.clone()));
        t.sort();
        t.dedup();
        t
    }

    fn set_health(&self, h: RelayHealth) {
        let differs = std::mem::replace(&mut *self.inner.health.lock(), h) != h;
        if differs {
            let v = {
                let mut shown = self.inner.shown.lock();
                shown.relay = h;
                shown.clone()
            };
            self.inner.deps.events.emit(EVENT, serde_json::to_value(&v).unwrap_or_default());
        }
    }

    /// Seal and publish, outside the state lock.
    /// False when something didn't go out.
    async fn flush(&self, out: Outgoing) -> bool {
        let mut all = true;
        for (ch, p) in out {
            if let Some(tap) = &self.inner.deps.tap {
                tap(&serde_json::to_string(&p).unwrap_or_default());
            }
            let bodies = match link::seal_packet(&ch, &p) {
                Ok(b) => b,
                Err(e) => {
                    tracing::debug!(kind = e.kind(), "couldn't seal a circle packet");
                    all = false;
                    continue;
                }
            };
            for b in bodies {
                match self.inner.relay.publish(&ch.topic, &b).await {
                    Ok(()) => self.set_health(RelayHealth::Ok),
                    Err(e) => {
                        self.set_health(e.into());
                        all = false;
                        break;
                    }
                }
            }
        }
        all
    }

    // ---- the relay subscription ----------------------------------------------------

    async fn pump(self) {
        let cfg = self.inner.deps.cfg;
        let mut topics = self.inner.topics.subscribe();
        let mut wait = cfg.retry;
        loop {
            let list = topics.borrow_and_update().clone();
            if list.is_empty() {
                if topics.changed().await.is_err() {
                    return;
                }
                continue;
            }
            // Everything the relay still holds the first time; after that,
            // what came since the last message seen.
            let since = self.inner.st.lock().await.saved.since.clone().unwrap_or_else(|| "all".into());
            match self.inner.relay.subscribe(&list, Some(&since)).await {
                Err(e) => {
                    self.set_health(e.into());
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        r = topics.changed() => if r.is_err() { return },
                    }
                    wait = (wait * 2).min(cfg.retry_max);
                }
                Ok(mut rx) => loop {
                    tokio::select! {
                        ev = rx.recv() => match ev {
                            Some(RelayEvent::Open) => {
                                wait = cfg.retry;
                                self.set_health(RelayHealth::Ok);
                                self.kick();
                            }
                            Some(RelayEvent::Message { id, topic, body }) => {
                                self.on_message(&topic, &body).await;
                                self.inner.st.lock().await.saved.since = Some(id);
                            }
                            None => {
                                self.set_health(RelayHealth::Unreachable);
                                tokio::time::sleep(wait).await;
                                wait = (wait * 2).min(cfg.retry_max);
                                break;
                            }
                        },
                        r = topics.changed() => {
                            if r.is_err() {
                                return;
                            }
                            break;
                        }
                    }
                },
            }
        }
    }

    async fn ticker(self) {
        let mut n: u64 = 0;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(self.inner.deps.cfg.tick) => { n += 1; }
                _ = self.inner.poke.notified() => {}
            }
            // Who's on this server, now and then (the doorbell).
            if n % 40 == 39 {
                self.refresh_doorbells().await;
            }
            self.tick().await;
        }
    }

    async fn refresh_doorbells(&self) {
        let s = self.settings();
        let on = s.playlists.load(Ordering::Relaxed) || s.blends.load(Ordering::Relaxed);
        let server = self.inner.deps.people.server().await;
        // Only when there's someone to hear from: a server or Home with others.
        let keys = if on && server.is_some() && !self.inner.deps.people.others().await.is_empty() { self.inner.deps.people.me().await } else { Vec::new() };
        let mut st = self.inner.st.lock().await;
        st.doorbells = match &server {
            Some(m) => keys.iter().map(|k| doorbell(m, k)).collect(),
            None => Vec::new(),
        };
        st.server = server;
        self.after(&st);
    }

    // ---- the background tick ---------------------------------------------------------

    pub async fn tick(&self) {
        let now = self.now();
        let cfg = self.inner.deps.cfg;
        let (first, max) = (cfg.resend_first.as_millis() as u64, cfg.resend_max.as_millis() as u64);
        let mut out: Outgoing = Vec::new();
        let mut st = self.inner.st.lock().await;
        // Owned playlists: what changed in Plex goes to everyone.
        let owned: Vec<usize> = st.saved.collabs.iter().enumerate().filter(|(_, c)| c.mine.is_some()).map(|(i, _)| i).collect();
        for i in owned {
            self.sync_owned(&mut st, i).await;
        }
        // Outboxes due.
        for c in &mut st.saved.collabs {
            if let Some(o) = &mut c.mine {
                for m in &mut o.members {
                    resend(&mut m.link, End::Owner, now, first, max, &mut out, &c.owner.id);
                }
                o.members.retain(|m| !(m.out && m.link.outbox.is_empty()));
                o.invites.retain(|i| i.expires + EXPIRED_KEPT > now);
            }
            if let Some(j) = &mut c.joined {
                let me = j.me.clone();
                resend(&mut j.link, End::Member, now, first, max, &mut out, &me);
            }
        }
        st.saved.collabs.retain(|c| {
            let owner_done = c.mine.as_ref().is_none_or(|o| !c.gone || !o.members.is_empty());
            let member_done = c.joined.as_ref().is_none_or(|j| !(j.leaving && j.link.outbox.is_empty()));
            owner_done && member_done && (c.mine.is_some() || c.joined.is_some())
        });
        // Invites waiting for an answer: ask again.
        st.joining.retain(|j| j.expires > now);
        for j in &mut st.joining {
            if now >= j.sent + first {
                j.sent = now;
                let m = Msg::Join { n: j.n.clone(), pake: seal::b64(&j.msg_a), name: (self.inner.deps.name)(), av: (self.inner.deps.avatar)() };
                out.push((j.up.clone(), Packet { f: j.n.clone(), c: 0, fl: 0, m }));
            }
        }
        // Blends: send this computer's taste when due, and make them again.
        let private = self.settings().private_session.load(Ordering::Relaxed);
        let unreleased = self.unreleased_ok();
        let taste_every = cfg.taste_every.as_millis() as u64;
        let due: Vec<usize> = st.saved.blends.iter().enumerate().filter(|(_, b)| b.sent_taste == 0 || now >= b.sent_taste + taste_every).map(|(i, _)| i).collect();
        if !due.is_empty() && !private {
            let t = self.inner.deps.library.taste(self.now_s(), unreleased).await;
            for i in due {
                let b = &mut st.saved.blends[i];
                b.sent_taste = now;
                out.push((b.channel(), group_packet(b, Msg::Taste { p: b.me.clone(), t: t.clone() })));
            }
        }
        for i in 0..st.saved.blends.len() {
            self.remake(&mut st, i, false).await;
        }
        st.saved.incoming.retain(|i| now < i.at + 7 * 86_400_000);
        self.after(&st);
        drop(st);
        self.flush(out).await;
    }

    /// Read an owned playlist and send what changed.
    async fn sync_owned(&self, st: &mut St, i: usize) {
        let now_s = self.now_s();
        let (playlist, gone) = {
            let c = &st.saved.collabs[i];
            (c.mine.as_ref().map(|o| o.playlist.clone()).unwrap_or_default(), c.gone)
        };
        if gone {
            return;
        }
        let rows = match self.inner.deps.playlists.rows(&playlist).await {
            Ok(r) => r,
            Err(e) => return tracing::debug!(kind = e.kind(), "couldn't read a shared playlist"),
        };
        let now = self.now();
        let c = &mut st.saved.collabs[i];
        let Some(rows) = rows else {
            // Deleted: members see it as unavailable.
            c.gone = true;
            if let Some(o) = &mut c.mine {
                for m in &mut o.members {
                    m.link.push(Msg::Gone, now);
                }
            }
            tracing::info!("a shared playlist was deleted");
            return;
        };
        let new_ids: Vec<String> = rows.rows.iter().map(|(_, t)| t.clone()).filter(|t| !c.songs.iter().any(|e| e.s.k.as_deref() == Some(t.as_str()))).collect();
        let mut who = if new_ids.is_empty() { HashMap::new() } else { self.inner.deps.library.describe(new_ids).await };
        let unreleased = self.unreleased_ok();
        // Unreleased songs stay home unless that's on.
        let c = &mut st.saved.collabs[i];
        for e in &c.songs {
            if let Some(k) = &e.s.k {
                who.entry(k.clone()).or_insert_with(|| e.s.clone());
            }
        }
        let kept: Vec<(String, String)> = rows.rows.iter().filter(|(_, t)| unreleased || !who.get(t).is_some_and(|s| s.u)).cloned().collect();
        let meta = c.title != rows.title || c.summary != rows.summary;
        c.title = rows.title.clone();
        c.summary = rows.summary.clone();
        let Some(o) = &mut c.mine else { return };
        let (list, delta) = collab::diff(&c.songs, c.v, &kept, &who, &mut o.claims, now_s);
        c.songs = list;
        if let Some(d) = delta.clone() {
            c.v = d.v;
        }
        if meta {
            c.v += 1;
            self.snapshot_all(c, now);
        } else if let Some(d) = delta
            && let Some(o) = &mut c.mine
        {
            for m in o.members.iter_mut().filter(|m| !m.out) {
                m.link.push(Msg::Delta(d.clone()), now);
            }
        }
    }

    /// Everyone gets the whole playlist (people or details changed).
    fn snapshot_all(&self, c: &mut Collab, now: u64) {
        let Some(o) = &c.mine else { return };
        c.ppl = member_list(o);
        let owner = c.owner.clone();
        let snaps: Vec<Snapshot> = o.members.iter().map(|m| snapshot(c, &owner, m.role)).collect();
        if let Some(o) = &mut c.mine {
            for (m, s) in o.members.iter_mut().zip(snaps) {
                if !m.out {
                    m.link.push(Msg::Snapshot(s), now);
                }
            }
        }
    }

    /// Make a Blend again when it's a new day or someone joined (`force`:
    /// now, whatever).
    async fn remake(&self, st: &mut St, i: usize, force: bool) {
        let now_s = self.now_s();
        let day = blend::day_of(now_s);
        let b = &st.saved.blends[i];
        let mut with: Vec<String> = b.active().filter(|m| m.taste.is_some()).map(|m| m.p.id.clone()).collect();
        with.sort();
        let joined_since = with.iter().any(|w| !b.made_with.contains(w));
        if !force && b.day == day && !joined_since && !(b.made.rows.is_empty() && !with.is_empty()) {
            return;
        }
        let unreleased = self.unreleased_ok();
        let mine = self.inner.deps.library.taste(now_s, unreleased).await;
        let b = &st.saved.blends[i];
        let mut members: Vec<(String, Taste)> = vec![(b.me.id.clone(), mine)];
        members.extend(b.active().filter_map(|m| m.taste.clone().map(|t| (m.p.id.clone(), t))));
        let hidden: HashSet<String> = b.hidden.iter().cloned().collect();
        let seed = (day as u64) ^ u64::from_str_radix(&b.g[..8.min(b.g.len())], 16).unwrap_or(0);
        let made = self.inner.deps.library.blend(members, hidden, unreleased, seed, b.me.id.clone()).await;
        let b = &mut st.saved.blends[i];
        b.made = made;
        b.day = day;
        b.made_with = with;
    }

    // ---- incoming --------------------------------------------------------------------

    async fn on_message(&self, topic: &str, body: &str) {
        let now = self.now();
        let mut out: Outgoing = Vec::new();
        let mut st = self.inner.st.lock().await;
        let st = &mut *st;
        // Which conversation it's on.
        let mut handled = false;
        for ci in 0..st.saved.collabs.len() {
            let c = &st.saved.collabs[ci];
            if let Some(o) = &c.mine {
                if let Some(ii) = o.invites.iter().position(|i| i.channels().0.topic == topic) {
                    let ch = o.invites[ii].channels().0;
                    if let Ok(Some(p)) = st.pieces.take(&ch, body, now) {
                        self.on_invite(st, Target::Collab(ci), ii, p, &mut out);
                    }
                    handled = true;
                    break;
                }
                if let Some(mi) = o.members.iter().position(|m| m.link.channels(End::Owner).1.topic == topic) {
                    let ch = o.members[mi].link.channels(End::Owner).1;
                    if let Ok(Some(p)) = st.pieces.take(&ch, body, now) {
                        self.on_member(st, ci, mi, p, &mut out).await;
                    }
                    handled = true;
                    break;
                }
            }
            if let Some(j) = &c.joined
                && j.link.channels(End::Member).1.topic == topic
            {
                let ch = j.link.channels(End::Member).1;
                if let Ok(Some(p)) = st.pieces.take(&ch, body, now) {
                    self.on_owner(st, ci, p, &mut out);
                }
                handled = true;
                break;
            }
        }
        if !handled {
            for bi in 0..st.saved.blends.len() {
                let b = &st.saved.blends[bi];
                if let Some(ii) = b.invites.iter().position(|i| i.channels().0.topic == topic) {
                    let ch = b.invites[ii].channels().0;
                    if let Ok(Some(p)) = st.pieces.take(&ch, body, now) {
                        self.on_invite(st, Target::Blend(bi), ii, p, &mut out);
                    }
                    handled = true;
                    break;
                }
                if b.channel().topic == topic {
                    let ch = b.channel();
                    if let Ok(Some(p)) = st.pieces.take(&ch, body, now) {
                        self.on_group(st, bi, p, &mut out);
                    }
                    handled = true;
                    break;
                }
            }
        }
        if !handled && let Some(ji) = st.joining.iter().position(|j| j.down.topic == topic) {
            let ch = st.joining[ji].down.clone();
            if let Ok(Some(p)) = st.pieces.take(&ch, body, now) {
                self.on_answer(st, ji, p, &mut out);
            }
            handled = true;
        }
        if !handled && let Some(d) = st.doorbells.iter().find(|d| d.topic == topic).cloned()
            && let Ok(Some(p)) = st.pieces.take(&d, body, now)
        {
            self.on_ring(st, p);
        }
        // Anything for a Blend that just got a new member: make it again.
        for i in 0..st.saved.blends.len() {
            self.remake(st, i, false).await;
        }
        self.after(st);
        if !out.is_empty() {
            self.flush(out).await;
        }
    }

    fn on_ring(&self, st: &mut St, p: Packet) {
        let Msg::Ring { link, from, kind, title } = p.m else { return };
        let ok = match kind {
            Kind::Playlist => invite::parse_as(&link, WORD_PLAYLIST).is_some() && self.settings().playlists.load(Ordering::Relaxed),
            Kind::Blend => invite::parse_as(&link, WORD_BLEND).is_some() && self.settings().blends.load(Ordering::Relaxed),
        };
        if !ok || st.saved.incoming.iter().any(|i| i.link == link) || st.saved.incoming.len() >= 32 {
            return;
        }
        let from = clean_name(&from);
        let title = clean_name(&title);
        let what = match kind {
            Kind::Playlist => format!("{from} invited you to the playlist {title}."),
            Kind::Blend => format!("{from} invited you to a Blend."),
        };
        st.saved.incoming.push(Incoming { id: rid(), from, kind, title, link, at: self.now() });
        self.notice(what);
    }

    /// Someone answered an invite this computer made.
    fn on_invite(&self, st: &mut St, target: Target, ii: usize, p: Packet, out: &mut Outgoing) {
        let now = self.now();
        let Msg::Join { n, pake, name, av } = p.m else { return };
        if n.len() > 64 {
            return;
        }
        let (title, inv, full, g, owner) = match target {
            Target::Collab(ci) => {
                let c = &st.saved.collabs[ci];
                let Some(o) = &c.mine else { return };
                (c.title.clone(), o.invites[ii].clone(), c.gone, c.g.clone(), c.owner.clone())
            }
            Target::Blend(bi) => {
                let b = &st.saved.blends[bi];
                (blend::title(&b.names()), b.invites[ii].clone(), b.active().count() + 1 >= blend::MAX_MEMBERS, b.g.clone(), b.me.clone())
            }
        };
        let (_, down) = inv.channels();
        let answer = |m: Msg| (down.clone(), Packet { f: owner.id.clone(), c: 0, fl: 0, m });
        match &inv.redeemed {
            Some((nonce, ans)) if *nonce == n => return out.push(answer(ans.clone())),
            Some(_) => return out.push(answer(Msg::Refused { n, why: Why::Used })),
            None if inv.expires <= now => return out.push(answer(Msg::Refused { n, why: Why::Expired })),
            None if full => {
                let why = if matches!(target, Target::Collab(_)) { Why::Gone } else { Why::Full };
                return out.push(answer(Msg::Refused { n, why }));
            }
            None => {}
        }
        let Some(msg_a) = seal::unb64(&pake) else { return };
        let Ok((msg_b, key)) = seal::pake_host(&seal::unb64(&inv.seed).unwrap_or_default(), &msg_a) else { return };
        let id = rid();
        let person = clean_person(Person { id: id.clone(), name, av });
        let welcome = match target {
            Target::Collab(_) => {
                let secret = seal::random32();
                let role = inv.role.unwrap_or(Role::Listener);
                (Welcome { kind: Kind::Playlist, g: g.clone(), id: id.clone(), link: seal::b64(&secret), owner: owner.clone(), title: title.clone(), role: Some(role), group: None, ppl: Vec::new() }, Some((secret, role)))
            }
            Target::Blend(bi) => {
                let b = &st.saved.blends[bi];
                let mut ppl = vec![b.me.clone()];
                ppl.extend(b.active().map(|m| m.p.clone()));
                (Welcome { kind: Kind::Blend, g: g.clone(), id: id.clone(), link: String::new(), owner: owner.clone(), title: title.clone(), role: None, group: Some(b.group.clone()), ppl }, None)
            }
        };
        let (welcome, collab) = welcome;
        let Ok(sealed) = seal::seal_with(&key, &welcome_aad(&n), &serde_json::to_vec(&welcome).unwrap_or_default()) else { return };
        let accepted = Msg::Accepted { n: n.clone(), pake: seal::b64(&msg_b), sealed };
        match target {
            Target::Collab(ci) => {
                let c = &mut st.saved.collabs[ci];
                let Some(o) = &mut c.mine else { return };
                o.invites[ii].redeemed = Some((n, accepted.clone()));
                let (secret, role) = collab.unwrap_or(([0; 32], Role::Listener));
                o.members.push(OwnedMember { p: person.clone(), role, link: Link::new(&secret), out: false });
                c.v += 1;
                self.snapshot_all(c, now);
                tracing::info!(role = role_word(role), "someone joined a shared playlist");
                self.notice(format!("{} joined {}.", person.name, c.title));
            }
            Target::Blend(bi) => {
                let b = &mut st.saved.blends[bi];
                b.invites[ii].redeemed = Some((n, accepted.clone()));
                if !b.members.iter().any(|m| m.p.id == id) {
                    b.members.push(BlendMember { p: person.clone(), taste: None, left: false });
                }
                // Everyone else hears who's new (and sends their taste again).
                out.push((b.channel(), group_packet(b, Msg::Joined { p: person.clone() })));
                b.sent_taste = 0;
                tracing::info!("someone joined a Blend");
                self.notice(format!("{} joined your Blend.", person.name));
            }
        }
        out.push(answer(accepted));
        self.kick();
    }

    /// An invite this computer answered was answered back.
    fn on_answer(&self, st: &mut St, ji: usize, p: Packet, _out: &mut Outgoing) {
        let j = &mut st.joining[ji];
        let result: AppResult<Welcome> = match p.m {
            Msg::Accepted { n, pake, sealed } if n == j.n => (|| {
                let state = j.pake.take().ok_or_else(|| AppError::Invalid("Already answered.".into()))?;
                let key = seal::pake_guest_finish(state, &seal::unb64(&pake).unwrap_or_default())?;
                let plain = seal::open_with(&key, &welcome_aad(&n), &sealed)?;
                serde_json::from_slice::<Welcome>(&plain).map_err(|_| AppError::Invalid("The answer couldn't be read.".into()))
            })(),
            Msg::Refused { n, why } if n == j.n => Err(AppError::Invalid(
                match why {
                    Why::Used => "That invite was already used. Ask for a new one.",
                    Why::Expired => "That invite has expired. Ask for a new one.",
                    Why::Full => "That Blend is full. A Blend takes up to 10 people.",
                    Why::Gone => "That playlist isn't shared any more.",
                }
                .into(),
            )),
            _ => return,
        };
        let j = st.joining.remove(ji);
        let res = result.and_then(|w| self.welcomed(st, j.kind, w));
        if let Err(e) = &res {
            self.notice(e.to_string());
        }
        if let Some(done) = j.done {
            let _ = done.send(res);
        }
    }

    /// In: keep what the welcome says.
    fn welcomed(&self, st: &mut St, kind: Kind, w: Welcome) -> AppResult<String> {
        if w.kind != kind || w.g.len() > 64 {
            return Err(AppError::Invalid("That invite didn't match.".into()));
        }
        let owner = clean_person(w.owner);
        let me_name = (self.inner.deps.name)();
        match kind {
            Kind::Playlist => {
                let secret = seal::key32(&w.link).ok_or_else(|| AppError::Invalid("That invite didn't match.".into()))?;
                let role = w.role.unwrap_or(Role::Listener);
                st.saved.collabs.retain(|c| c.g != w.g || c.mine.is_some());
                let mut link = Link::new(&secret);
                link.push(Msg::Hello { name: me_name }, self.now());
                st.saved.collabs.push(Collab {
                    g: w.g.clone(),
                    title: clean_name(&w.title),
                    summary: None,
                    owner: owner.clone(),
                    public: false,
                    v: 0,
                    songs: Vec::new(),
                    ppl: Vec::new(),
                    gone: false,
                    in_library: true,
                    mine: None,
                    joined: Some(Joined { me: w.id, role, link, pending: Vec::new(), leaving: false }),
                });
                tracing::info!("joined a shared playlist");
                self.kick();
                Ok(format!("{SHARED_PREFIX}{}", w.g))
            }
            Kind::Blend => {
                let group = w.group.filter(|g| seal::key32(g).is_some()).ok_or_else(|| AppError::Invalid("That invite didn't match.".into()))?;
                st.saved.blends.retain(|b| b.g != w.g);
                let me = self.me_person(&w.id);
                let members = w.ppl.into_iter().filter(|p| p.id != w.id).take(blend::MAX_MEMBERS - 1).map(|p| BlendMember { p: clean_person(p), taste: None, left: false }).collect();
                st.saved.blends.push(BlendState {
                    g: w.g.clone(),
                    me,
                    group,
                    members,
                    invites: Vec::new(),
                    hidden: Vec::new(),
                    made: Made::default(),
                    day: 0,
                    made_with: Vec::new(),
                    guard: HashMap::new(),
                    next: 1,
                    sent_taste: 0,
                });
                tracing::info!("joined a Blend");
                self.kick();
                Ok(format!("{BLEND_PREFIX}{}", w.g))
            }
        }
    }

    /// A packet from a member of a playlist this computer owns.
    async fn on_member(&self, st: &mut St, ci: usize, mi: usize, p: Packet, out: &mut Outgoing) {
        let now = self.now();
        let (owner_id, playlist) = {
            let c = &st.saved.collabs[ci];
            (c.owner.id.clone(), c.mine.as_ref().map(|o| o.playlist.clone()).unwrap_or_default())
        };
        let (msg, fresh, ack_to) = {
            let Some(o) = &mut st.saved.collabs[ci].mine else { return };
            let m = &mut o.members[mi];
            if p.f != m.p.id {
                return;
            }
            if let Msg::Ack { upto, also } = &p.m {
                m.link.acked(*upto, also, now);
                return;
            }
            let fresh = m.link.accept(p.c, p.fl, now);
            let ack = Packet { f: owner_id.clone(), c: 0, fl: 0, m: m.link.ack() };
            (p.m, fresh, (m.link.channels(End::Owner).0, ack))
        };
        out.push(ack_to);
        if !fresh {
            return;
        }
        match msg {
            Msg::Hello { name } => {
                let c = &mut st.saved.collabs[ci];
                let owner = c.owner.clone();
                let Some(o) = &mut c.mine else { return };
                let name = clean_name(&name);
                if !name.is_empty() {
                    o.members[mi].p.name = name;
                }
                let role = o.members[mi].role;
                c.ppl = member_list(o);
                let s = snapshot(c, &owner, role);
                if let Some(o) = &mut c.mine {
                    o.members[mi].link.push(Msg::Snapshot(s), now);
                }
            }
            Msg::Edit { n, op } => {
                let (role, member) = {
                    let o = st.saved.collabs[ci].mine.as_ref().expect("owned");
                    (o.members[mi].role, o.members[mi].p.clone())
                };
                let checked = collab::check_edit(role, &member.id, &op, &st.saved.collabs[ci].songs);
                let result: Result<(), String> = match checked {
                    Err(why) => Err(why),
                    Ok(()) => match &op {
                        EditOp::Add { songs } => {
                            let unreleased = self.unreleased_ok();
                            let songs: Vec<Ident> = songs.iter().filter(|s| unreleased || !s.u).cloned().collect();
                            let found = self.inner.deps.library.resolve(songs.clone()).await;
                            let ids: Vec<String> = found.iter().flatten().cloned().collect();
                            let owner_name = st.saved.collabs[ci].owner.name.clone();
                            if ids.is_empty() {
                                Err(match songs.first() {
                                    Some(s) if songs.len() == 1 => format!("{owner_name} doesn't have {} on their server.", s.n),
                                    _ => format!("{owner_name} doesn't have those songs on their server."),
                                })
                            } else {
                                match self.inner.deps.playlists.add(&playlist, ids.clone()).await {
                                    Ok(()) => {
                                        if let Some(o) = &mut st.saved.collabs[ci].mine {
                                            o.claims.extend(ids.into_iter().map(|t| (t, member.id.clone())));
                                        }
                                        Ok(())
                                    }
                                    Err(e) => Err(format!("That didn't work on {owner_name}'s side: {e}")),
                                }
                            }
                        }
                        EditOp::Remove { keys } => {
                            let have: Vec<String> = keys.iter().filter(|k| st.saved.collabs[ci].songs.iter().any(|e| &e.key == *k)).cloned().collect();
                            if have.is_empty() {
                                Ok(())
                            } else {
                                self.inner.deps.playlists.remove(&playlist, have).await.map_err(|e| e.to_string())
                            }
                        }
                    },
                };
                // The change first (so it lands with the confirmation), then the answer.
                if result.is_ok() {
                    self.sync_owned(st, ci).await;
                }
                let applied = Msg::Applied { n, ok: result.is_ok(), why: result.err() };
                if let Some(o) = &mut st.saved.collabs[ci].mine {
                    o.members[mi].link.push(applied, now);
                }
            }
            Msg::Leave => {
                let c = &mut st.saved.collabs[ci];
                let public = c.public;
                let Some(o) = &mut c.mine else { return };
                let m = &mut o.members[mi];
                tracing::info!("someone left a shared playlist");
                if public {
                    m.role = Role::Follower;
                } else {
                    m.out = true;
                    m.link.outbox.clear();
                }
                c.v += 1;
                self.snapshot_all(c, now);
            }
            _ => {}
        }
        self.kick();
    }

    /// A packet from the owner of a playlist shared with this computer.
    fn on_owner(&self, st: &mut St, ci: usize, p: Packet, out: &mut Outgoing) {
        let now = self.now();
        let c = &mut st.saved.collabs[ci];
        if p.f != c.owner.id {
            return;
        }
        let Some(j) = &mut c.joined else { return };
        if let Msg::Ack { upto, also } = &p.m {
            j.link.acked(*upto, also, now);
            return;
        }
        let fresh = j.link.accept(p.c, p.fl, now);
        out.push((j.link.channels(End::Member).0, Packet { f: j.me.clone(), c: 0, fl: 0, m: j.link.ack() }));
        if !fresh || j.leaving {
            return;
        }
        match p.m {
            Msg::Snapshot(s) => {
                c.v = s.v;
                c.title = clean_name(&s.title);
                c.summary = s.sum.map(|s| s.chars().take(500).collect());
                c.public = s.public;
                c.owner.name = clean_person(s.owner).name;
                c.ppl = s.ppl.into_iter().map(|m| Member { p: clean_person(m.p), r: m.r }).collect();
                c.songs = s.songs.into_iter().take(collab::MAX_SONGS).collect();
                j.role = s.you;
            }
            Msg::Delta(d) => {
                let Delta { .. } = d;
                if !collab::apply(&mut c.songs, &mut c.v, &d) {
                    // Out of step: ask for all of it.
                    j.link.push(Msg::Hello { name: (self.inner.deps.name)() }, now);
                }
            }
            Msg::Applied { n, ok, why } => {
                j.pending.retain(|e| e.n != n);
                if !ok {
                    self.notice(why.unwrap_or_else(|| "That change couldn't be made.".into()));
                }
            }
            Msg::Gone => {
                c.gone = true;
                j.pending.clear();
                self.notice(format!("{} deleted {}.", c.owner.name, c.title));
            }
            Msg::Removed => {
                c.gone = true;
                j.pending.clear();
                j.role = Role::Listener;
                self.notice(format!("You were taken off {}.", c.title));
            }
            _ => {}
        }
        self.kick();
    }

    /// A packet on a Blend's shared channel.
    fn on_group(&self, st: &mut St, bi: usize, p: Packet, _out: &mut Outgoing) {
        let b = &mut st.saved.blends[bi];
        if p.f == b.me.id || p.f.len() > 32 {
            return;
        }
        let last = b.guard.entry(p.f.clone()).or_insert(0);
        if p.c <= *last {
            return;
        }
        *last = p.c;
        match p.m {
            Msg::Taste { p: who, t } => {
                if who.id != p.f {
                    return;
                }
                let who = clean_person(who);
                let room = b.active().count() + 1 < blend::MAX_MEMBERS;
                match b.members.iter_mut().find(|m| m.p.id == who.id) {
                    Some(m) if m.left => {}
                    Some(m) => {
                        m.p = who;
                        m.taste = Some(trim_taste(t));
                    }
                    None if room => b.members.push(BlendMember { p: who, taste: Some(trim_taste(t)), left: false }),
                    None => {}
                }
            }
            Msg::Joined { p: who } => {
                let who = clean_person(who);
                if who.id != b.me.id && !b.members.iter().any(|m| m.p.id == who.id) && b.active().count() + 1 < blend::MAX_MEMBERS {
                    b.members.push(BlendMember { p: who, taste: None, left: false });
                }
                // Someone new needs everyone's taste.
                b.sent_taste = 0;
                self.kick();
            }
            Msg::Left { id } => {
                if id != p.f {
                    return;
                }
                if let Some(m) = b.members.iter_mut().find(|m| m.p.id == id) {
                    m.left = true;
                    m.taste = None;
                }
                if b.active().count() == 0 {
                    let title = blend::title(&b.names());
                    st.saved.blends.remove(bi);
                    self.notice(format!("Everyone else left {title}, so it's gone."));
                }
            }
            _ => {}
        }
    }

    // ---- views -------------------------------------------------------------------------

    fn build_view(&self, st: &St) -> CircleView {
        let now = self.now();
        CircleView {
            relay: *self.inner.health.lock(),
            playlists: st.saved.collabs.iter().filter_map(summary).collect(),
            blends: st.saved.blends.iter().map(blend_summary).collect(),
            incoming: st
                .saved
                .incoming
                .iter()
                .map(|i| IncomingView { id: i.id.clone(), from: i.from.clone(), kind: i.kind, title: i.title.clone(), at: i.at })
                .collect(),
            joining: st.joining.iter().filter(|j| j.expires > now).count() as u32,
        }
    }

    // ---- what the UI asks for --------------------------------------------------------

    fn need(&self, kind: Kind) -> AppResult<()> {
        let s = self.settings();
        let (on, what) = match kind {
            Kind::Playlist => (s.playlists.load(Ordering::Relaxed), "Collaborative playlists are off in Settings."),
            Kind::Blend => (s.blends.load(Ordering::Relaxed), "Blends are off in Settings."),
        };
        if on { Ok(()) } else { Err(AppError::Invalid(what.into())) }
    }

    fn new_invite(&self, role: Option<Role>, word: &str) -> Invite {
        let now = self.now();
        let hours = self.settings().invite_hours.load(Ordering::Relaxed).clamp(1, 24 * 7) as u64;
        let expires = now + hours * 3_600_000;
        let relay = (self.inner.relay_base != self.inner.deps.default_relay).then(|| self.inner.relay_base.clone());
        let token = Token { session: seal::random16(), secret: seal::random16(), expires: (expires / 1000) as u32, relay };
        Invite { seed: seal::b64(&token.seed()), expires, link: token.link_as(word), role, redeemed: None }
    }

    /// The shared summary of a playlist (yours: its Plex id; shared with
    /// you: `shared:<group>`). None when it isn't shared.
    pub async fn shared(&self, id: &str) -> Option<SharedSummary> {
        let st = self.inner.st.lock().await;
        find_collab(&st.saved, id).and_then(|i| summary(&st.saved.collabs[i]))
    }

    /// A link to invite someone to your playlist as `role`.
    pub async fn invite_link(&self, playlist_id: &str, role: Role) -> AppResult<InviteLink> {
        self.need(Kind::Playlist)?;
        if !matches!(role, Role::Collaborator | Role::Listener) {
            return Err(AppError::Invalid("Invite people as collaborators or listeners.".into()));
        }
        let rows = self.inner.deps.playlists.rows(playlist_id).await?.ok_or_else(|| AppError::NotFound("This playlist was deleted.".into()))?;
        let inv = self.new_invite(Some(role), WORD_PLAYLIST);
        let link = InviteLink { link: inv.link.clone(), expires_at: inv.expires };
        let mut st = self.inner.st.lock().await;
        let i = match st.saved.collabs.iter().position(|c| c.mine.as_ref().is_some_and(|o| o.playlist == playlist_id)) {
            Some(i) => i,
            None => {
                let me = self.me_person(&rid());
                st.saved.collabs.push(Collab {
                    g: hex(&seal::random16()),
                    title: rows.title.clone(),
                    summary: rows.summary.clone(),
                    owner: me,
                    public: false,
                    v: 0,
                    songs: Vec::new(),
                    ppl: Vec::new(),
                    gone: false,
                    in_library: true,
                    mine: Some(Owned { playlist: playlist_id.into(), members: Vec::new(), invites: Vec::new(), claims: Vec::new() }),
                    joined: None,
                });
                let i = st.saved.collabs.len() - 1;
                self.sync_owned(&mut st, i).await;
                i
            }
        };
        let c = &mut st.saved.collabs[i];
        c.owner = self.me_person(&c.owner.id);
        let o = c.mine.as_mut().expect("owned");
        o.invites.retain(|i| i.expires + EXPIRED_KEPT > self.now());
        while o.invites.len() >= MAX_INVITES {
            o.invites.remove(0);
        }
        o.invites.push(inv);
        self.after(&st);
        Ok(link)
    }

    /// Ring someone on this server or Home with an invite.
    async fn ring(&self, key: &str, link: &str, kind: Kind, title: &str) -> AppResult<()> {
        let server = { self.inner.st.lock().await.server.clone() };
        let server = match server {
            Some(s) => s,
            None => self.inner.deps.people.server().await.ok_or_else(|| AppError::Invalid("Connect to your Plex server first.".into()))?,
        };
        let ch = doorbell(&server, key);
        let p = Packet { f: String::new(), c: 0, fl: 0, m: Msg::Ring { link: link.into(), from: (self.inner.deps.name)(), kind, title: title.into() } };
        if let Some(tap) = &self.inner.deps.tap {
            tap(&serde_json::to_string(&p).unwrap_or_default());
        }
        for b in link::seal_packet(&ch, &p)? {
            if let Err(e) = self.inner.relay.publish(&ch.topic, &b).await {
                self.set_health(e.into());
                return Err(relay_err());
            }
        }
        self.set_health(RelayHealth::Ok);
        Ok(())
    }

    /// Invite someone on this server or Home straight to your playlist.
    pub async fn invite_person(&self, playlist_id: &str, key: &str, role: Role) -> AppResult<()> {
        let link = self.invite_link(playlist_id, role).await?;
        let title = self.shared(playlist_id).await.map(|s| s.title).unwrap_or_default();
        self.ring(key, &link.link, Kind::Playlist, &title).await
    }

    /// People this computer can invite directly.
    pub async fn people(&self) -> Vec<Account> {
        self.inner.deps.people.others().await
    }

    /// Join from an invite link: waits a little for the answer, else keeps
    /// asking in the background while the owner is offline.
    pub async fn join(&self, raw: &str) -> AppResult<JoinResult> {
        let (token, kind) = if let Some(t) = invite::parse_as(raw, WORD_PLAYLIST) {
            (t, Kind::Playlist)
        } else if let Some(t) = invite::parse_as(raw, WORD_BLEND) {
            (t, Kind::Blend)
        } else {
            return Err(AppError::Invalid("That isn't a playlist or Blend invite.".into()));
        };
        self.need(kind)?;
        let now = self.now();
        if u64::from(token.expires) * 1000 <= now {
            return Err(AppError::Invalid("That invite has expired. Ask for a new one.".into()));
        }
        if token.relay.as_deref().unwrap_or(&self.inner.deps.default_relay) != self.inner.relay_base {
            return Err(AppError::Invalid("That invite uses another relay. Set the same relay under Listening sessions in Settings.".into()));
        }
        let seed = token.seed();
        let (up, down) = invite_channels(&seed);
        let (rx, packet) = {
            let mut st = self.inner.st.lock().await;
            if st.saved.blends.len() >= 32 && kind == Kind::Blend {
                return Err(AppError::Invalid("You're in as many Blends as Platter keeps.".into()));
            }
            if st.joining.iter().any(|j| j.up.topic == up.topic) {
                return Ok(JoinResult::Waiting { kind });
            }
            let (pake, msg_a) = seal::pake_guest_start(&seed);
            let n = rid();
            let (tx, rx) = oneshot::channel();
            let m = Msg::Join { n: n.clone(), pake: seal::b64(&msg_a), name: (self.inner.deps.name)(), av: clean_avatar((self.inner.deps.avatar)()) };
            st.joining.push(Joining { kind, n: n.clone(), msg_a, pake: Some(pake), up: up.clone(), down, expires: u64::from(token.expires) * 1000, sent: now, done: Some(tx) });
            st.saved.incoming.retain(|i| i.link != raw.trim());
            self.after(&st);
            (rx, Packet { f: n, c: 0, fl: 0, m })
        };
        // Listen for the answer before asking.
        self.wait_for_topic(&invite_channels(&seed).1.topic).await;
        if !self.flush(vec![(up.clone(), packet)]).await {
            let mut st = self.inner.st.lock().await;
            st.joining.retain(|j| j.up.topic != up.topic);
            self.after(&st);
            return Err(AppError::Network("Couldn't reach the relay to join. Try again in a while.".into()));
        }
        match tokio::time::timeout(self.inner.deps.cfg.answer_timeout, rx).await {
            Ok(Ok(Ok(id))) => {
                self.notice(match kind {
                    Kind::Playlist => "You joined the playlist.".into(),
                    Kind::Blend => "You joined the Blend.".into(),
                });
                Ok(JoinResult::Joined { id, kind })
            }
            Ok(Ok(Err(e))) => Err(e),
            _ => Ok(JoinResult::Waiting { kind }),
        }
    }

    /// Give the subscription a moment to include a new topic.
    async fn wait_for_topic(&self, topic: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            if self.inner.topics.borrow().iter().any(|t| t == topic) && *self.inner.health.lock() == RelayHealth::Ok {
                tokio::time::sleep(Duration::from_millis(50)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub async fn dismiss(&self, id: &str) {
        let mut st = self.inner.st.lock().await;
        st.saved.incoming.retain(|i| i.id != id);
        self.after(&st);
    }

    /// Join the invite that arrived on the doorbell.
    pub async fn accept(&self, id: &str) -> AppResult<JoinResult> {
        let link = {
            let st = self.inner.st.lock().await;
            st.saved.incoming.iter().find(|i| i.id == id).map(|i| i.link.clone()).ok_or_else(|| AppError::NotFound("That invite isn't here any more.".into()))?
        };
        self.join(&link).await
    }

    /// The owner's view of their shared playlist.
    pub async fn info(&self, playlist_id: &str) -> AppResult<Option<SharedInfo>> {
        let st = self.inner.st.lock().await;
        let Some(i) = st.saved.collabs.iter().position(|c| c.mine.as_ref().is_some_and(|o| o.playlist == playlist_id)) else { return Ok(None) };
        let c = &st.saved.collabs[i];
        let Some(s) = summary(c) else { return Ok(None) };
        let people: HashMap<&str, &PersonView> = s.people.iter().map(|p| (p.id.as_str(), p)).collect();
        let added_by = c
            .songs
            .iter()
            .filter_map(|e| e.by.as_deref().and_then(|b| people.get(b)).map(|p| (e.key.clone(), (*p).clone())))
            .collect();
        Ok(Some(SharedInfo { summary: s, added_by }))
    }

    /// A playlist shared with you, as the songs you can play here.
    pub async fn page(&self, id: &str) -> AppResult<SharedPage> {
        let (c, s) = {
            let st = self.inner.st.lock().await;
            let i = find_collab(&st.saved, id).ok_or_else(|| AppError::NotFound("This playlist isn't shared with you any more.".into()))?;
            let c = st.saved.collabs[i].clone();
            let s = summary(&c).ok_or_else(|| AppError::NotFound("This playlist isn't shared with you any more.".into()))?;
            (c, s)
        };
        let j = c.joined.as_ref();
        let me = j.map(|j| j.me.clone()).unwrap_or_default();
        let role = j.map_or(Role::Owner, |j| j.role);
        let removing: HashSet<&String> = j.iter().flat_map(|j| &j.pending).filter_map(|e| if let EditOp::Remove { keys } = &e.op { Some(keys) } else { None }).flatten().collect();
        let mut entries: Vec<(Entry, bool)> = c.songs.iter().filter(|e| !removing.contains(&e.key)).map(|e| (e.clone(), false)).collect();
        for e in j.iter().flat_map(|j| &j.pending) {
            if let EditOp::Add { songs } = &e.op {
                entries.extend(songs.iter().enumerate().map(|(i, s)| (Entry { key: format!("pending:{}:{i}", e.n), s: s.clone(), by: Some(me.clone()), at: 0 }, true)));
            }
        }
        let found = self.inner.deps.library.resolve(entries.iter().map(|(e, _)| e.s.clone()).collect()).await;
        let tracks = self.inner.deps.library.tracks(found.iter().flatten().cloned().collect()).await;
        let by_id: HashMap<&str, &Track> = tracks.iter().map(|t| (t.id.as_str(), t)).collect();
        let people: HashMap<&str, &PersonView> = s.people.iter().map(|p| (p.id.as_str(), p)).collect();
        let mut duration = 0;
        let rows = entries
            .into_iter()
            .zip(found)
            .map(|((e, pending), f)| {
                let track = f.as_deref().and_then(|id| by_id.get(id)).map(|t| (*t).clone());
                duration += track.as_ref().map_or(e.s.d, |t| t.duration_ms);
                SharedRow {
                    key: e.key.clone(),
                    track,
                    title: e.s.n.clone(),
                    artist: e.s.a.clone(),
                    album: e.s.al.clone(),
                    duration_ms: e.s.d,
                    release_group_id: e.s.rg.clone(),
                    by: e.by.as_deref().and_then(|b| people.get(b)).map(|p| (*p).clone()),
                    added_at: e.at,
                    removable: !pending && role == Role::Collaborator && e.by.as_deref() == Some(me.as_str()),
                    pending,
                }
            })
            .collect();
        Ok(SharedPage { summary: s, description: c.summary.clone(), duration_ms: duration, rows })
    }

    fn joined_mut<'a>(st: &'a mut St, id: &str) -> AppResult<&'a mut Collab> {
        let i = find_collab(&st.saved, id).ok_or_else(|| AppError::NotFound("This playlist isn't shared with you any more.".into()))?;
        Ok(&mut st.saved.collabs[i])
    }

    /// Add songs to a playlist shared with you (as a collaborator): they go
    /// to the owner, now or when they're next online.
    pub async fn add(&self, id: &str, track_ids: Vec<String>) -> AppResult<()> {
        if track_ids.is_empty() || track_ids.len() > collab::MAX_ADD {
            return Err(AppError::Invalid("Add up to 100 songs at a time.".into()));
        }
        let who = self.inner.deps.library.describe(track_ids.clone()).await;
        let unreleased = self.unreleased_ok();
        let songs: Vec<Ident> = track_ids.iter().filter_map(|t| who.get(t).cloned()).filter(|s| unreleased || !s.u).collect();
        if songs.is_empty() {
            return Err(AppError::Invalid(if who.values().any(|s| s.u) {
                "Unreleased songs aren't shared. You can change that in Settings.".into()
            } else {
                "Those songs couldn't be added.".into()
            }));
        }
        let now = self.now();
        let mut st = self.inner.st.lock().await;
        let c = Self::joined_mut(&mut st, id)?;
        if c.gone {
            return Err(AppError::Invalid("This playlist isn't available any more.".into()));
        }
        let Some(j) = &mut c.joined else { return Err(AppError::Invalid("This is your own playlist.".into())) };
        if j.role != Role::Collaborator {
            return Err(AppError::Invalid("Only collaborators can add songs to this playlist.".into()));
        }
        let n = rid();
        let op = EditOp::Add { songs };
        j.link.push(Msg::Edit { n: n.clone(), op: op.clone() }, now);
        j.pending.push(PendingEdit { n, op });
        self.after(&st);
        drop(st);
        self.kick();
        Ok(())
    }

    /// Remove songs you added from a playlist shared with you.
    pub async fn remove(&self, id: &str, keys: Vec<String>) -> AppResult<()> {
        let now = self.now();
        let mut st = self.inner.st.lock().await;
        let c = Self::joined_mut(&mut st, id)?;
        let songs = c.songs.clone();
        let Some(j) = &mut c.joined else { return Err(AppError::Invalid("This is your own playlist.".into())) };
        let op = EditOp::Remove { keys };
        collab::check_edit(j.role, &j.me, &op, &songs).map_err(AppError::Invalid)?;
        let n = rid();
        j.link.push(Msg::Edit { n: n.clone(), op: op.clone() }, now);
        j.pending.push(PendingEdit { n, op });
        self.after(&st);
        drop(st);
        self.kick();
        Ok(())
    }

    /// Leave a playlist shared with you. On a public one you keep seeing it.
    pub async fn leave(&self, id: &str) -> AppResult<()> {
        let now = self.now();
        let mut st = self.inner.st.lock().await;
        let c = Self::joined_mut(&mut st, id)?;
        let gone = c.gone;
        let public = c.public;
        let Some(j) = &mut c.joined else { return Err(AppError::Invalid("You own this playlist.".into())) };
        if gone {
            // Nothing to tell anyone.
            let g = c.g.clone();
            st.saved.collabs.retain(|c| c.g != g || c.mine.is_some());
        } else if public {
            j.link.push(Msg::Leave, now);
            j.role = Role::Follower;
            j.pending.clear();
        } else {
            j.link.outbox.clear();
            j.link.push(Msg::Leave, now);
            j.leaving = true;
        }
        self.after(&st);
        drop(st);
        self.kick();
        Ok(())
    }

    /// Show or hide a playlist shared with you in your library (you stay in it).
    pub async fn set_in_library(&self, id: &str, on: bool) -> AppResult<()> {
        let mut st = self.inner.st.lock().await;
        Self::joined_mut(&mut st, id)?.in_library = on;
        self.after(&st);
        Ok(())
    }

    fn owned_mut<'a>(st: &'a mut St, playlist_id: &str) -> AppResult<&'a mut Collab> {
        st.saved
            .collabs
            .iter_mut()
            .find(|c| c.mine.as_ref().is_some_and(|o| o.playlist == playlist_id))
            .ok_or_else(|| AppError::NotFound("This playlist isn't shared.".into()))
    }

    /// Change someone's role on your playlist.
    pub async fn set_role(&self, playlist_id: &str, person: &str, role: Role) -> AppResult<()> {
        if !matches!(role, Role::Collaborator | Role::Listener) {
            return Err(AppError::Invalid("Choose collaborator or listener.".into()));
        }
        let now = self.now();
        let mut st = self.inner.st.lock().await;
        let c = Self::owned_mut(&mut st, playlist_id)?;
        let o = c.mine.as_mut().expect("owned");
        let m = o.members.iter_mut().find(|m| m.p.id == person && !m.out).ok_or_else(|| AppError::NotFound("They're not on this playlist any more.".into()))?;
        m.role = role;
        c.v += 1;
        self.snapshot_all(c, now);
        self.after(&st);
        drop(st);
        self.kick();
        Ok(())
    }

    /// Take someone off your playlist (they're told).
    pub async fn remove_person(&self, playlist_id: &str, person: &str) -> AppResult<()> {
        let now = self.now();
        let mut st = self.inner.st.lock().await;
        let c = Self::owned_mut(&mut st, playlist_id)?;
        let o = c.mine.as_mut().expect("owned");
        let m = o.members.iter_mut().find(|m| m.p.id == person && !m.out).ok_or_else(|| AppError::NotFound("They're not on this playlist any more.".into()))?;
        m.out = true;
        m.link.outbox.clear();
        m.link.push(Msg::Removed, now);
        c.v += 1;
        self.snapshot_all(c, now);
        self.after(&st);
        drop(st);
        self.kick();
        Ok(())
    }

    /// Private or public. Made private, people who left (and still see it)
    /// lose it.
    pub async fn set_public(&self, playlist_id: &str, public: bool) -> AppResult<()> {
        let now = self.now();
        let mut st = self.inner.st.lock().await;
        let c = Self::owned_mut(&mut st, playlist_id)?;
        c.public = public;
        if !public && let Some(o) = &mut c.mine {
            for m in o.members.iter_mut().filter(|m| m.role == Role::Follower && !m.out) {
                m.out = true;
                m.link.outbox.clear();
                m.link.push(Msg::Removed, now);
            }
        }
        c.v += 1;
        self.snapshot_all(c, now);
        self.after(&st);
        drop(st);
        self.kick();
        Ok(())
    }

    // ---- Blends -----------------------------------------------------------------

    /// A new Blend with you in it: an invite goes to each person picked
    /// (people on your server or Home), and the link is for anyone else.
    pub async fn blend_create(&self, keys: Vec<String>) -> AppResult<BlendCreated> {
        self.need(Kind::Blend)?;
        if keys.len() + 1 > blend::MAX_MEMBERS {
            return Err(AppError::Invalid("A Blend takes up to 10 people, you included.".into()));
        }
        let g = hex(&seal::random16());
        let me = self.me_person(&rid());
        let first = self.new_invite(None, WORD_BLEND);
        let mut rings = Vec::new();
        {
            let mut st = self.inner.st.lock().await;
            let mut invites = vec![first.clone()];
            for k in &keys {
                let inv = self.new_invite(None, WORD_BLEND);
                rings.push((k.clone(), inv.link.clone()));
                invites.push(inv);
            }
            st.saved.blends.push(BlendState {
                g: g.clone(),
                me: me.clone(),
                group: seal::b64(&seal::random32()),
                members: Vec::new(),
                invites,
                hidden: Vec::new(),
                made: Made::default(),
                day: 0,
                made_with: Vec::new(),
                guard: HashMap::new(),
                next: 1,
                sent_taste: 0,
            });
            let i = st.saved.blends.len() - 1;
            self.remake(&mut st, i, true).await;
            self.after(&st);
        }
        for (k, l) in rings {
            if let Err(e) = self.ring(&k, &l, Kind::Blend, &me.name).await {
                tracing::debug!(kind = e.kind(), "a Blend invite couldn't be sent");
            }
        }
        self.kick();
        Ok(BlendCreated { id: format!("{BLEND_PREFIX}{g}"), invite: InviteLink { link: first.link, expires_at: first.expires } })
    }

    fn blend_index(st: &St, id: &str) -> AppResult<usize> {
        let g = id.strip_prefix(BLEND_PREFIX).unwrap_or(id);
        st.saved.blends.iter().position(|b| b.g == g).ok_or_else(|| AppError::NotFound("This Blend isn't here any more.".into()))
    }

    /// Another invite link for a Blend.
    pub async fn blend_invite(&self, id: &str) -> AppResult<InviteLink> {
        self.need(Kind::Blend)?;
        let inv = self.new_invite(None, WORD_BLEND);
        let mut st = self.inner.st.lock().await;
        let i = Self::blend_index(&st, id)?;
        let b = &mut st.saved.blends[i];
        if b.active().count() + 1 >= blend::MAX_MEMBERS {
            return Err(AppError::Invalid("This Blend is full. A Blend takes up to 10 people.".into()));
        }
        b.invites.retain(|i| i.expires > self.now() && i.redeemed.is_none());
        while b.invites.len() >= MAX_INVITES {
            b.invites.remove(0);
        }
        let out = InviteLink { link: inv.link.clone(), expires_at: inv.expires };
        b.invites.push(inv);
        self.after(&st);
        Ok(out)
    }

    pub async fn blend_invite_person(&self, id: &str, key: &str) -> AppResult<()> {
        let link = self.blend_invite(id).await?;
        let title = {
            let st = self.inner.st.lock().await;
            let i = Self::blend_index(&st, id)?;
            st.saved.blends[i].me.name.clone()
        };
        self.ring(key, &link.link, Kind::Blend, &title).await
    }

    pub async fn blend_page(&self, id: &str) -> AppResult<BlendPage> {
        let b = {
            let st = self.inner.st.lock().await;
            st.saved.blends[Self::blend_index(&st, id)?].clone()
        };
        let s = blend_summary(&b);
        let people: HashMap<&str, &PersonView> = s.people.iter().map(|p| (p.id.as_str(), p)).collect();
        // Someone who left stays until the next day's Blend.
        let gone_by = |id: &str| -> PersonView {
            b.members.iter().find(|m| m.p.id == id).map(|m| person_view(&m.p, None, false)).unwrap_or_else(|| person_view(&b.me, None, true))
        };
        let view = |id: &str| people.get(id).map(|p| (*p).clone()).unwrap_or_else(|| gone_by(id));
        let tracks = self.inner.deps.library.tracks(b.made.rows.iter().map(|r| r.id.clone()).collect()).await;
        let by_id: HashMap<&str, &Track> = tracks.iter().map(|t| (t.id.as_str(), t)).collect();
        let rows: Vec<BlendRowView> = b
            .made
            .rows
            .iter()
            .filter(|r| !b.hidden.contains(&r.id))
            .filter_map(|r| by_id.get(r.id.as_str()).map(|t| BlendRowView { track: (*t).clone(), by: view(&r.by) }))
            .collect();
        let duration_ms = rows.iter().map(|r| r.track.duration_ms).sum();
        let missing = b
            .made
            .missing
            .iter()
            .map(|m| MissingView { title: m.s.n.clone(), artist: m.s.a.clone(), album: m.s.al.clone(), release_group_id: m.s.rg.clone(), by: view(&m.by) })
            .collect();
        Ok(BlendPage { summary: BlendSummary { track_count: rows.len() as u32, ..s }, rows, missing, duration_ms })
    }

    /// Hide a song in this Blend: it leaves now and doesn't come back.
    pub async fn blend_hide(&self, id: &str, track_id: &str) -> AppResult<()> {
        let mut st = self.inner.st.lock().await;
        let i = Self::blend_index(&st, id)?;
        let b = &mut st.saved.blends[i];
        if !b.hidden.iter().any(|h| h == track_id) {
            b.hidden.push(track_id.into());
        }
        self.after(&st);
        Ok(())
    }

    /// Make it again now.
    pub async fn blend_refresh(&self, id: &str) -> AppResult<BlendPage> {
        {
            let mut st = self.inner.st.lock().await;
            let i = Self::blend_index(&st, id)?;
            self.remake(&mut st, i, true).await;
            self.after(&st);
        }
        self.blend_page(id).await
    }

    /// Leave a Blend: it's deleted here and this computer stops sharing.
    pub async fn blend_leave(&self, id: &str) -> AppResult<()> {
        let out = {
            let mut st = self.inner.st.lock().await;
            let i = Self::blend_index(&st, id)?;
            let mut b = st.saved.blends.remove(i);
            let me = b.me.id.clone();
            let p = group_packet(&mut b, Msg::Left { id: me });
            self.after(&st);
            vec![(b.channel(), p)]
        };
        tracing::info!("left a Blend");
        self.flush(out).await;
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Target {
    Collab(usize),
    Blend(usize),
}

fn trim_taste(mut t: Taste) -> Taste {
    t.ar.truncate(blend::TOP_ARTISTS);
    t.so.truncate(blend::TOP_SONGS);
    t.fav.truncate(blend::FAVOURITES);
    t
}

fn group_packet(b: &mut BlendState, m: Msg) -> Packet {
    let c = b.next;
    b.next += 1;
    Packet { f: b.me.id.clone(), c, fl: 0, m }
}

fn resend(l: &mut Link, end: End, now: u64, first: u64, max: u64, out: &mut Outgoing, from: &str) {
    if !l.due(now) {
        return;
    }
    let ch = l.channels(end).0;
    let fl = l.floor();
    for o in &l.outbox {
        out.push((ch.clone(), Packet { f: from.to_string(), c: o.c, fl, m: o.m.clone() }));
    }
    l.sent(now, first, max);
}

fn find_collab(s: &Saved, id: &str) -> Option<usize> {
    match id.strip_prefix(SHARED_PREFIX) {
        Some(g) => s.collabs.iter().position(|c| c.g == g && c.joined.is_some()),
        None => s.collabs.iter().position(|c| c.mine.as_ref().is_some_and(|o| o.playlist == id)),
    }
}

fn person_view(p: &Person, role: Option<Role>, you: bool) -> PersonView {
    PersonView { id: p.id.clone(), name: p.name.clone(), avatar: p.av.clone(), role, you, added: 0 }
}

fn summary(c: &Collab) -> Option<SharedSummary> {
    let counts = collab::added_counts(&c.songs);
    let added = |id: &str| counts.get(id).copied().unwrap_or(0);
    if let Some(o) = &c.mine {
        let members: Vec<&OwnedMember> = o.members.iter().filter(|m| !m.out && m.role != Role::Follower).collect();
        let mut people = vec![PersonView { added: added(&c.owner.id), ..person_view(&c.owner, Some(Role::Owner), true) }];
        people.extend(members.iter().map(|m| PersonView { added: added(&m.p.id), ..person_view(&m.p, Some(m.role), false) }));
        return Some(SharedSummary {
            id: o.playlist.clone(),
            group: c.g.clone(),
            title: c.title.clone(),
            owner: people[0].clone(),
            role: Role::Owner,
            people,
            public: c.public,
            gone: c.gone,
            in_library: true,
            waiting: 0,
            owner_unreachable: false,
            track_count: c.songs.len() as u32,
        });
    }
    let j = c.joined.as_ref()?;
    if j.leaving {
        return None;
    }
    let owner = PersonView { added: added(&c.owner.id), ..person_view(&c.owner, Some(Role::Owner), false) };
    let mut people = vec![owner.clone()];
    people.extend(c.ppl.iter().map(|m| PersonView { added: added(&m.p.id), ..person_view(&m.p, Some(m.r), m.p.id == j.me) }));
    Some(SharedSummary {
        id: format!("{SHARED_PREFIX}{}", c.g),
        group: c.g.clone(),
        title: c.title.clone(),
        owner,
        role: j.role,
        people,
        public: c.public,
        gone: c.gone,
        in_library: c.in_library,
        waiting: j.pending.len() as u32,
        owner_unreachable: !j.link.outbox.is_empty() && j.link.tries >= 3,
        track_count: c.songs.len() as u32,
    })
}

fn blend_summary(b: &BlendState) -> BlendSummary {
    let names = b.names();
    let mut people = vec![person_view(&b.me, None, true)];
    people.extend(b.active().map(|m| person_view(&m.p, None, false)));
    BlendSummary {
        id: format!("{BLEND_PREFIX}{}", b.g),
        title: blend::title(&names),
        description: blend::description(&names),
        people,
        track_count: b.made.rows.iter().filter(|r| !b.hidden.contains(&r.id)).count() as u32,
    }
}
