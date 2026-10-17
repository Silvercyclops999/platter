//! Listen along: one person hosts, people with access to the same Plex
//! server follow, and nobody learns anyone else's network address.
//!
//! Everyone connects only to a relay (an ntfy server, [`relay`]), never to
//! each other. Every message is sealed end to end ([`seal`]); the relay sees
//! random topic names and ciphertext. Invites are random single-use tokens
//! ([`invite`]). Guests play from their own access to the host's server, so
//! songs travel as rating keys, with who each song is (MusicBrainz id,
//! title, artist, length) for guests without the host's copy, who play the
//! same song from their own library when they have it ([`matching`]). No
//! stream URL, path, server address or Plex token is ever sent.
//!
//! Channels (each a topic and a key from a secret only participants hold):
//! - per invite: `invite up` (guest → host) and `invite down` (host →
//!   guest), from the token. The first guest to answer agrees a key with the
//!   host (SPAKE2 over the invite secret) and gets, sealed with it, its own
//!   secret and the broadcast secret. The token is then used up.
//! - per guest: `guest up` and `guest down`, from the guest's secret.
//! - `broadcast` (host → all guests), from a secret that changes whenever
//!   someone leaves or is removed, so they can't keep listening in.
//!
//! The host publishes the state when it changes (and every
//! [`ListenConfig::heartbeat`]), only while someone is listening; ntfy.sh
//! allows 250 messages a day per address. A guest that hears nothing from
//! the host for a heartbeat plus [`ListenConfig::grace`] shows the host as
//! gone, and picks up again if the host comes back.

pub mod invite;
pub mod matching;
pub mod relay;
pub mod seal;
pub mod wire;

#[cfg(test)]
pub(crate) mod fake_ntfy;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use serde::Serialize;
use tokio::sync::{Notify, watch};
use tokio::time::Instant;

use crate::error::{AppError, AppResult};
use crate::plex::state::EventSink;
use crate::remote::peer::auth::{hex, now_ms};
use crate::sync::keycopy::Qr;
use invite::Token;
use relay::{Relay, RelayError, RelayEvent, RelayFactory, RelayHealth};
use seal::{Channel, Envelope, ReplayGuard, Sender};
use matching::Found;
use wire::{Msg, Op, Person, Queued, Song, State, Welcome, Why};

pub const EVENT: &str = "listen://state";
/// A guest's request was turned down, or something else worth a toast.
pub const EVENT_NOTICE: &str = "listen://notice";
/// The relay used unless the user sets their own.
pub const DEFAULT_RELAY: &str = "https://ntfy.sh";

/// Guests one session takes.
pub const MAX_GUESTS: usize = 10;
/// A guest this far from the host's spot seeks to it.
pub const DRIFT_MS: u64 = 1500;
/// Songs after the current one a guest is sent (fewer if the message would
/// be too big).
pub const UP_NEXT: usize = 50;
/// Songs a guest may add in one go.
pub const MAX_ADD: usize = 100;
/// Invites a host keeps open at once.
pub const MAX_INVITES: usize = 16;
/// "Invites expire after" choices, hours.
pub const INVITE_HOURS: [u32; 3] = [1, 24, 168];
pub const NAME_MAX: usize = 40;
const HOST_ID: &str = "host";
/// Assumed round trip through the relay, for following the host's spot.
const LATENCY_MS: u64 = 500;
/// The host's position moving this much more than time did is a seek.
const SEEK_JUMP_MS: u64 = 3000;

/// Timing (shorter in tests).
#[derive(Debug, Clone, Copy)]
pub struct ListenConfig {
    /// How often the host looks at its player, and a guest at the clock.
    pub watch: Duration,
    /// The host sends the state at least this often while people listen.
    pub heartbeat: Duration,
    /// How much longer than a heartbeat a guest waits before the host counts
    /// as gone.
    pub grace: Duration,
    /// A guest says it's still there this often.
    pub guest_ping: Duration,
    /// A guest not heard from for this long has left.
    pub guest_timeout: Duration,
    /// How long a join waits for the host's answer.
    pub answer_timeout: Duration,
    /// First and longest wait before reconnecting to the relay.
    pub retry: Duration,
    pub retry_max: Duration,
    /// The host answers one guest's "send me the state" at most this often.
    pub hello_every: Duration,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            watch: Duration::from_secs(1),
            heartbeat: Duration::from_secs(180),
            grace: Duration::from_secs(30),
            guest_ping: Duration::from_secs(180),
            guest_timeout: Duration::from_secs(600),
            answer_timeout: Duration::from_secs(20),
            retry: Duration::from_secs(2),
            retry_max: Duration::from_secs(60),
            hello_every: Duration::from_secs(5),
        }
    }
}

/// The settings listen along reads.
#[derive(Clone)]
pub struct ListenSettings {
    /// "Let others change what's playing" (off: guests can only add).
    pub guests_control: Arc<AtomicBool>,
    /// The relay URL; empty means [`DEFAULT_RELAY`].
    pub relay: Arc<RwLock<String>>,
    /// "Invites expire after", hours.
    pub invite_hours: Arc<AtomicU32>,
    /// The name others see; empty means the Plex display name.
    pub name: Arc<RwLock<String>>,
    /// "Hide unreleased titles from guests": an unreleased song goes out
    /// without its title and artist.
    pub hide_unreleased: Arc<AtomicBool>,
}

impl Default for ListenSettings {
    fn default() -> Self {
        Self {
            guests_control: Arc::new(AtomicBool::new(true)),
            relay: Arc::default(),
            invite_hours: Arc::new(AtomicU32::new(24)),
            name: Arc::default(),
            hide_unreleased: Arc::new(AtomicBool::new(false)),
        }
    }
}

// ---- the player and the server ------------------------------------------------

/// What playback is doing here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlayerNow {
    pub track_id: Option<String>,
    pub position_ms: u64,
    pub duration_ms: u64,
    pub playing: bool,
    /// Songs the user queued (play before the rest).
    pub user_queue: Vec<String>,
    /// Everything after this song: the user queue, then the rest.
    pub up_next: Vec<String>,
    /// The queue item ids of `up_next`, same order.
    pub up_next_items: Vec<String>,
}

/// The local player, as a session needs it.
#[async_trait]
pub trait ListenPlayer: Send + Sync {
    async fn now(&self) -> AppResult<PlayerNow>;
    /// Play these songs from the first, at a spot, playing or paused.
    async fn play(&self, ids: Vec<String>, position_ms: u64, play: bool) -> AppResult<()>;
    async fn seek(&self, position_ms: u64) -> AppResult<()>;
    async fn pause(&self) -> AppResult<()>;
    async fn resume(&self) -> AppResult<()>;
    async fn next(&self) -> AppResult<()>;
    async fn previous(&self) -> AppResult<()>;
    /// Add to the end of the user queue.
    async fn queue_add(&self, ids: Vec<String>) -> AppResult<()>;
    /// Empty the user queue.
    async fn queue_clear(&self) -> AppResult<()>;
    async fn queue_move(&self, item_id: String, to_index: usize) -> AppResult<()>;
}

/// The Plex server this computer plays from.
#[async_trait]
pub trait ServerId: Send + Sync {
    async fn machine_id(&self) -> Option<String>;
}

/// The library, as a session needs it.
#[async_trait]
pub trait ListenLibrary: Send + Sync {
    /// Who these songs are (the host's side); unknown ids are left out.
    async fn songs(&self, ids: Vec<String>) -> HashMap<String, Song>;
    /// The host's own copy plays here (this library has that id).
    async fn has(&self, id: &str) -> bool;
    /// The same song here by the multi-server rules; never an unreleased one.
    async fn find(&self, song: &Song) -> Option<Found>;
}

/// What sharing the host's Plex server did ("Let them listen from my
/// server").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferOutcome {
    /// plex.tv sent the invite.
    Invited,
    /// They can already reach the host's server.
    AlreadyShared,
    /// Why not, in words for the host.
    Failed(String),
}

/// Plex sharing, as listen along needs it. The app shares through plex.tv
/// with the host's chosen music libraries only (`social::sharing`); a guest
/// finds the invite in Platter. [`NoOffer`] stands in when there's none.
#[async_trait]
pub trait ServerOffer: Send + Sync {
    /// Host: "Let them listen from my server" is on (and this account owns
    /// the server it plays from).
    async fn offered(&self) -> bool;
    /// Host: share my music with this Plex user.
    async fn invite(&self, plex_user: &str) -> OfferOutcome;
    /// Guest: my Plex username, when "Accept invites inside Platter" is on.
    async fn me(&self) -> Option<String>;
    /// Guest: the host says they shared; look for the invite.
    async fn shared(&self, host: &str);
}

pub struct NoOffer;

#[async_trait]
impl ServerOffer for NoOffer {
    async fn offered(&self) -> bool {
        false
    }
    async fn invite(&self, _: &str) -> OfferOutcome {
        OfferOutcome::Failed("Sharing isn't available here.".into())
    }
    async fn me(&self) -> Option<String> {
        None
    }
    async fn shared(&self, _: &str) {}
}

/// One step a guest takes to match the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Load { id: String, position_ms: u64, play: bool },
    /// Make the user queue these songs.
    SetQueue(Vec<String>),
    Seek(u64),
    Pause,
    Resume,
}

/// Where the host is by the time its state arrives: half the round trip
/// later while playing, never past the end.
pub fn target_ms(st: &State, rtt_ms: u64) -> u64 {
    let at = if st.pl { st.p + rtt_ms / 2 } else { st.p };
    if st.d > 0 { at.min(st.d) } else { at }
}

/// What a guest does to match the host.
pub fn plan(local: &PlayerNow, st: &State, rtt_ms: u64) -> Vec<Step> {
    let queue: Vec<String> = st.q.iter().map(|q| q.k.clone()).collect();
    let Some(track) = st.t.clone() else {
        return if local.playing { vec![Step::Pause] } else { Vec::new() };
    };
    let target = target_ms(st, rtt_ms);
    if local.track_id.as_deref() != Some(track.as_str()) {
        // The guest moved on to the next song a moment before the host did.
        let ahead = local.track_id.is_some() && queue.first() == local.track_id.as_ref() && st.d.saturating_sub(target) <= 2 * DRIFT_MS;
        if ahead {
            return Vec::new();
        }
        return vec![Step::Load { id: track, position_ms: target, play: st.pl }, Step::SetQueue(queue)];
    }
    let mut steps = Vec::new();
    if local.user_queue != queue {
        steps.push(Step::SetQueue(queue));
    }
    if local.position_ms.abs_diff(target) > DRIFT_MS {
        steps.push(Step::Seek(target));
    }
    match (st.pl, local.playing) {
        (true, false) => steps.push(Step::Resume),
        (false, true) => steps.push(Step::Pause),
        _ => {}
    }
    steps
}

async fn apply(player: &dyn ListenPlayer, steps: Vec<Step>) -> AppResult<()> {
    for s in steps {
        match s {
            Step::Load { id, position_ms, play } => player.play(vec![id], position_ms, play).await?,
            Step::SetQueue(ids) => {
                player.queue_clear().await?;
                if !ids.is_empty() {
                    player.queue_add(ids).await?;
                }
            }
            Step::Seek(ms) => player.seek(ms).await?,
            Step::Pause => player.pause().await?,
            Step::Resume => player.resume().await?,
        }
    }
    Ok(())
}

// ---- what the UI sees -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    None,
    Host,
    Guest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    /// Joining, or waiting for the host's first state.
    Connecting,
    Live,
    /// Nothing from the host for a while.
    HostGone,
    /// The host ended the session.
    Ended,
    /// The host removed this guest.
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonView {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub host: bool,
    pub me: bool,
    /// The host's view: this guest has no copy of the song playing.
    pub cant_hear: bool,
    /// The host's view: where "Let them listen from my server" is for them.
    pub offer: Option<OfferState>,
}

/// "Let them listen from my server", for one guest (the host's view).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "message")]
pub enum OfferState {
    /// Asked their Platter who they are on Plex.
    Asking,
    /// Sending the invite through Plex.
    Inviting,
    /// Invited; they accept in their Platter.
    Invited,
    /// They can already reach the server.
    AlreadyShared,
    /// Their Platter doesn't take invites; share from Settings instead.
    NoInvites,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedView {
    pub track_id: String,
    /// Who added it (a person id), when a guest did.
    pub by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteView {
    pub link: String,
    pub qr: Option<Qr>,
    /// Unix ms.
    pub expires_at: u64,
}

/// A guest's view: the host's song has no copy here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingSong {
    /// None when the host hides unreleased titles (or sent none).
    pub title: Option<String>,
    pub artist: Option<String>,
    pub unreleased: bool,
}

/// `listen_state` and the `listen://state` event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenView {
    pub role: Role,
    pub session_id: Option<String>,
    pub host_name: Option<String>,
    /// The current invite (host only; one use, then ask for a new one).
    pub invite: Option<InviteView>,
    /// Everyone in the session, host first.
    pub people: Vec<PersonView>,
    /// The names of everyone else (for the strip's one line).
    pub listeners: Vec<String>,
    pub guests_can_control: bool,
    /// Who added the song playing, and the shared up next.
    pub now_by: Option<String>,
    pub queue: Vec<QueuedView>,
    pub status: Option<SessionStatus>,
    pub relay: RelayHealth,
    /// Host: the names of guests who can't hear the song playing.
    pub cant_hear: Vec<String>,
    /// Guest: the song playing isn't in this library; the player waits for
    /// the next one.
    pub missing: Option<MissingSong>,
    /// Guest: playing a copy of another length ("another version").
    pub another_version: bool,
    /// Host: "Let them listen from my server" can be offered.
    pub offer_server: bool,
}

impl ListenView {
    pub fn none() -> Self {
        Self {
            role: Role::None,
            session_id: None,
            host_name: None,
            invite: None,
            people: Vec::new(),
            listeners: Vec::new(),
            guests_can_control: false,
            now_by: None,
            queue: Vec::new(),
            status: None,
            relay: RelayHealth::Ok,
            cant_hear: Vec::new(),
            missing: None,
            another_version: false,
            offer_server: false,
        }
    }
}

/// For Discord: the session and how many are in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyInfo {
    pub id: String,
    pub size: u32,
    pub max: u32,
}

/// `listen://notice`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub message: String,
}

// ---- state ------------------------------------------------------------------

fn clean_name(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(NAME_MAX).collect::<String>().trim().to_string()
}

fn ok_id(s: &str) -> bool {
    !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// A song id: a rating key, with its server tag in front (`crate::sid`).
fn ok_song(s: &str) -> bool {
    match crate::sid::split(s) {
        (Some(_), k) => ok_id(k) && s.len() <= 64,
        (None, k) => ok_id(k),
    }
}

fn invite_channels(t: &Token) -> (Channel, Channel) {
    let seed = t.seed();
    (Channel::new(&seed, "invite up"), Channel::new(&seed, "invite down"))
}

fn guest_channels(secret: &[u8; 32]) -> (Channel, Channel, [u8; 32]) {
    (Channel::new(secret, "guest up"), Channel::new(secret, "guest down"), seal::derive(secret, "guest tag"))
}

fn welcome_aad(n: &str) -> String {
    format!("platter-listen-v1 welcome\n{n}")
}

/// Parse an invite link (for deep links).
pub fn parse_invite(raw: &str) -> Option<Token> {
    invite::parse(raw)
}

/// The QR code for any text.
pub fn qr_for(text: &str) -> Option<Qr> {
    crate::sync::keycopy::qr_text(text)
}

struct HostInvite {
    token: Token,
    up: Channel,
    down: Channel,
    link: String,
    expires_ms: u64,
    /// Answered: the attempt's nonce and the answer (sent again if asked).
    redeemed: Option<(String, Msg)>,
}

struct HostGuest {
    id: String,
    name: String,
    up: Channel,
    down: Channel,
    tag: [u8; 32],
    seen: Instant,
    hello_at: Option<Instant>,
    /// The song (host id) they said they have no copy of.
    cant: Option<String>,
    offer: Option<OfferState>,
}

/// The last state broadcast.
struct Sent {
    sig: State,
    at: Instant,
}

struct Hosting {
    session: [u8; 16],
    relay: Arc<dyn Relay>,
    /// In invites when it isn't the default relay.
    relay_in_link: Option<String>,
    srv: String,
    invites: Vec<HostInvite>,
    guests: Vec<HostGuest>,
    bcast: Channel,
    epoch: u32,
    bcast_secret: [u8; 32],
    replay: ReplayGuard,
    /// Songs guests added and who, oldest first.
    added: Vec<(String, String)>,
    topics: watch::Sender<Vec<String>>,
    health: RelayHealth,
    last: Option<Sent>,
    /// The state as last built (for the host's own view).
    shown: Option<State>,
    /// Who songs are, looked up once each (None: not in the library).
    songs: HashMap<String, Option<Song>>,
}

impl Hosting {
    fn topic_list(&self) -> Vec<String> {
        self.invites.iter().map(|i| i.up.topic.clone()).chain(self.guests.iter().map(|g| g.up.topic.clone())).collect()
    }

    fn refresh_topics(&self) {
        let t = self.topic_list();
        self.topics.send_if_modified(|old| std::mem::replace(old, t.clone()) != t);
    }

    /// A new broadcast secret; the messages that hand it to each guest.
    fn rekey(&mut self) -> Vec<(Channel, Msg)> {
        self.bcast_secret = seal::random32();
        self.epoch += 1;
        self.bcast = Channel::new(&self.bcast_secret, "broadcast");
        self.last = None;
        let b = seal::b64(&self.bcast_secret);
        self.guests.iter().map(|g| (g.down.clone(), Msg::Rekey { b: b.clone(), e: self.epoch })).collect()
    }

    fn current_invite(&self, now: u64) -> Option<&HostInvite> {
        self.invites.iter().rev().find(|i| i.redeemed.is_none() && i.expires_ms > now)
    }
}

struct Following {
    session: [u8; 16],
    relay: Arc<dyn Relay>,
    me: String,
    host: Person,
    up: Channel,
    down: Channel,
    tag: [u8; 32],
    bcast: Channel,
    epoch: u32,
    status: SessionStatus,
    health: RelayHealth,
    replay: ReplayGuard,
    last: Option<State>,
    heard: Instant,
    topics: watch::Sender<Vec<String>>,
    /// The host's songs and their copies here (None: none).
    found: HashMap<String, Option<Found>>,
    /// The song playing has no copy here.
    missing: Option<MissingSong>,
    other_version: bool,
    /// The song we last told the host we can't hear.
    told: Option<String>,
}

impl Following {
    fn topic_list(&self) -> Vec<String> {
        vec![self.bcast.topic.clone(), self.down.topic.clone()]
    }
}

enum Mode {
    Idle,
    Joining { session: String },
    Host(Hosting),
    Guest(Following),
}

/// Sees each message before it is sealed (tests).
pub type Tap = Arc<dyn Fn(&str) + Send + Sync>;

/// Who this computer is, and the shared parts.
pub struct ListenDeps {
    pub player: Arc<dyn ListenPlayer>,
    pub relays: Arc<dyn RelayFactory>,
    pub events: Arc<dyn EventSink>,
    pub server: Arc<dyn ServerId>,
    /// Who songs are, and finding them here.
    pub library: Arc<dyn ListenLibrary>,
    /// Sharing the host's Plex server with a guest who can't hear a song.
    pub offer: Arc<dyn ServerOffer>,
    /// The name others see.
    pub name: Arc<dyn Fn() -> String + Send + Sync>,
    pub settings: ListenSettings,
    /// [`DEFAULT_RELAY`] in the app.
    pub default_relay: String,
    pub cfg: ListenConfig,
    /// Sees every message before it's sealed (tests).
    pub tap: Option<Tap>,
}

struct Inner {
    deps: ListenDeps,
    mode: Mutex<Mode>,
    /// Bumped whenever a session starts or stops; older tasks quit.
    generation: AtomicU64,
    counter: AtomicU64,
    /// One message out at a time, so counters reach the relay in order.
    send_lock: tokio::sync::Mutex<()>,
    /// One set of steps at a time while following.
    follow_lock: tokio::sync::Mutex<()>,
    /// Look at the player now (a guest asked for something).
    poke: Notify,
    party: watch::Sender<Option<PartyInfo>>,
    /// The host's "Let them listen from my server" (read when hosting starts
    /// and on [`ListenAlong::settings_changed`]).
    offered: AtomicBool,
    /// When each guest last asked for something (the host's side).
    op_rate: Mutex<HashMap<String, Vec<std::time::Instant>>>,
}

/// Requests one guest may make per [`GUEST_OPS_WINDOW`].
const GUEST_OPS: usize = 20;
const GUEST_OPS_WINDOW: Duration = Duration::from_secs(10);

/// The same song by title and artist (folded), or by MusicBrainz id.
fn same_song(here: &Song, theirs: &Song) -> bool {
    if here.mb.is_some() && here.mb == theirs.mb {
        return true;
    }
    let both = |a: &Option<String>, b: &Option<String>| match (a, b) {
        (Some(a), Some(b)) => !a.trim().is_empty() && crate::db::merge::norm(a) == crate::db::merge::norm(b),
        _ => false,
    };
    both(&here.n, &theirs.n) && both(&here.a, &theirs.a)
}

/// Listen along on this computer. Cheap to clone.
#[derive(Clone)]
pub struct ListenAlong {
    inner: Arc<Inner>,
}

fn relay_err(e: RelayError) -> AppError {
    match e {
        RelayError::Limited => AppError::Network("The listen along relay is busy. Try again in a while, or use your own relay in Settings.".into()),
        _ => AppError::Network("Couldn't reach the listen along relay.".into()),
    }
}

impl ListenAlong {
    pub fn new(deps: ListenDeps) -> Self {
        Self {
            inner: Arc::new(Inner {
                deps,
                mode: Mutex::new(Mode::Idle),
                generation: AtomicU64::new(0),
                counter: AtomicU64::new(0),
                send_lock: tokio::sync::Mutex::new(()),
                follow_lock: tokio::sync::Mutex::new(()),
                poke: Notify::new(),
                party: watch::channel(None).0,
                offered: AtomicBool::new(false),
                op_rate: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// The session and its size whenever it changes (Discord shows it).
    pub fn subscribe_party(&self) -> watch::Receiver<Option<PartyInfo>> {
        self.inner.party.subscribe()
    }

    fn cfg(&self) -> ListenConfig {
        self.inner.deps.cfg
    }

    fn current(&self, generation: u64) -> bool {
        self.inner.generation.load(Ordering::SeqCst) == generation
    }

    fn bump(&self) -> u64 {
        self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn my_name(&self) -> String {
        let n = clean_name(&(self.inner.deps.name)());
        if n.is_empty() { "Someone".into() } else { n }
    }

    fn can_control(&self) -> bool {
        self.inner.deps.settings.guests_control.load(Ordering::Relaxed)
    }

    // ---- the view -----------------------------------------------------------

    /// What the UI shows now.
    pub fn view(&self) -> ListenView {
        let mode = self.inner.mode.lock();
        match &*mode {
            Mode::Idle => ListenView::none(),
            Mode::Joining { session } => ListenView {
                role: Role::Guest,
                session_id: Some(session.clone()),
                status: Some(SessionStatus::Connecting),
                relay: RelayHealth::Connecting,
                ..ListenView::none()
            },
            Mode::Host(h) => {
                let me = self.my_name();
                let mut people = vec![PersonView { id: HOST_ID.into(), name: me.clone(), avatar: None, host: true, me: true, cant_hear: false, offer: None }];
                people.extend(h.guests.iter().map(|g| PersonView {
                    id: g.id.clone(),
                    name: g.name.clone(),
                    avatar: None,
                    host: false,
                    me: false,
                    cant_hear: g.cant.is_some(),
                    offer: g.offer.clone(),
                }));
                let invite = h.current_invite(now_ms()).map(|i| InviteView { link: i.link.clone(), qr: qr_for(&i.link), expires_at: i.expires_ms });
                ListenView {
                    role: Role::Host,
                    session_id: Some(hex(&h.session)),
                    host_name: Some(me),
                    invite,
                    listeners: h.guests.iter().map(|g| g.name.clone()).collect(),
                    people,
                    guests_can_control: self.can_control(),
                    now_by: h.shown.as_ref().and_then(|s| s.tby.clone()),
                    queue: h.shown.as_ref().map(queue_view).unwrap_or_default(),
                    status: Some(SessionStatus::Live),
                    relay: h.health,
                    cant_hear: h.guests.iter().filter(|g| g.cant.is_some()).map(|g| g.name.clone()).collect(),
                    missing: None,
                    another_version: false,
                    offer_server: self.inner.offered.load(Ordering::Relaxed),
                }
            }
            Mode::Guest(f) => {
                let ppl: Vec<Person> = match &f.last {
                    Some(st) if !st.ppl.is_empty() => st.ppl.clone(),
                    _ => vec![f.host.clone(), Person { id: f.me.clone(), name: self.my_name(), av: None }],
                };
                let people: Vec<PersonView> = ppl
                    .iter()
                    .map(|p| PersonView { id: p.id.clone(), name: p.name.clone(), avatar: None, host: p.id == HOST_ID, me: p.id == f.me, cant_hear: false, offer: None })
                    .collect();
                ListenView {
                    role: Role::Guest,
                    session_id: Some(hex(&f.session)),
                    host_name: Some(f.host.name.clone()),
                    invite: None,
                    listeners: people.iter().filter(|p| !p.me).map(|p| p.name.clone()).collect(),
                    people,
                    guests_can_control: f.last.as_ref().is_some_and(|s| s.ctl),
                    now_by: f.last.as_ref().and_then(|s| s.tby.clone()),
                    // Songs with a copy here show as that copy (the queue panel lists it).
                    queue: f
                        .last
                        .as_ref()
                        .map(queue_view)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|q| match f.found.get(&q.track_id) {
                            Some(Some(here)) => QueuedView { track_id: here.id.clone(), ..q },
                            _ => q,
                        })
                        .collect(),
                    status: Some(f.status),
                    relay: f.health,
                    cant_hear: Vec::new(),
                    missing: if f.status == SessionStatus::Live { f.missing.clone() } else { None },
                    another_version: f.status == SessionStatus::Live && f.other_version,
                    offer_server: false,
                }
            }
        }
    }

    fn changed(&self) {
        let v = self.view();
        let party = match v.role {
            Role::Host => v.session_id.clone().map(|id| PartyInfo { id, size: v.people.len() as u32, max: MAX_GUESTS as u32 + 1 }),
            _ => None,
        };
        self.inner.party.send_if_modified(|p| std::mem::replace(p, party.clone()) != party);
        self.inner.deps.events.emit(EVENT, serde_json::to_value(&v).unwrap_or_default());
    }

    fn notice(&self, message: &str) {
        self.inner.deps.events.emit(EVENT_NOTICE, serde_json::to_value(Notice { message: message.into() }).unwrap_or_default());
    }

    fn set_health(&self, generation: u64, h: RelayHealth) {
        let differs = {
            let mut mode = self.inner.mode.lock();
            if !self.current(generation) {
                return;
            }
            let slot = match &mut *mode {
                Mode::Host(x) => &mut x.health,
                Mode::Guest(x) => &mut x.health,
                _ => return,
            };
            std::mem::replace(slot, h) != h
        };
        if differs {
            if h != RelayHealth::Ok {
                tracing::info!(health = ?h, "listen along relay");
            }
            self.changed();
        }
    }

    // ---- sending ------------------------------------------------------------

    /// Seal and publish one message.
    async fn send(&self, relay: &Arc<dyn Relay>, ch: &Channel, r: Sender, from: &str, m: Msg, tags: &[[u8; 32]]) -> Result<(), RelayError> {
        let _one = self.inner.send_lock.lock().await;
        let env = Envelope { r, f: from.into(), c: self.inner.counter.fetch_add(1, Ordering::SeqCst) + 1, x: now_ms() + seal::TTL_MS, m };
        if let Some(tap) = &self.inner.deps.tap {
            tap(&serde_json::to_string(&env).unwrap_or_default());
        }
        let text = seal::seal(ch, &env, tags).map_err(|_| RelayError::Refused(0))?;
        relay.publish(&ch.topic, &text).await
    }

    async fn send_host(&self, generation: u64, relay: &Arc<dyn Relay>, ch: &Channel, m: Msg, tags: &[[u8; 32]]) {
        match self.send(relay, ch, Sender::Host, HOST_ID, m, tags).await {
            Ok(()) => self.set_health(generation, RelayHealth::Ok),
            Err(e) => self.set_health(generation, e.into()),
        }
    }

    // ---- the relay subscription ---------------------------------------------

    /// Listen on the session's topics until it ends, reconnecting as needed.
    async fn pump(self, generation: u64, relay: Arc<dyn Relay>, mut topics: watch::Receiver<Vec<String>>) {
        let cfg = self.cfg();
        let mut since: Option<String> = None;
        let mut wait = cfg.retry;
        loop {
            if !self.current(generation) {
                return;
            }
            let list = topics.borrow_and_update().clone();
            if list.is_empty() {
                if topics.changed().await.is_err() {
                    return;
                }
                continue;
            }
            match relay.subscribe(&list, since.as_deref()).await {
                Err(e) => {
                    self.set_health(generation, e.into());
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
                                self.set_health(generation, RelayHealth::Ok);
                                self.on_open(generation).await;
                            }
                            Some(RelayEvent::Message { id, topic, body }) => {
                                since = Some(id);
                                self.on_message(generation, &topic, &body).await;
                            }
                            None => {
                                self.set_health(generation, RelayHealth::Unreachable);
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
                    if !self.current(generation) {
                        return;
                    }
                },
            }
        }
    }

    async fn on_open(&self, generation: u64) {
        // A guest (re)connected: ask for the state.
        let hello = match &*self.inner.mode.lock() {
            Mode::Guest(f) if self.current(generation) => Some((f.relay.clone(), f.up.clone(), f.me.clone())),
            _ => None,
        };
        if let Some((relay, up, me)) = hello
            && let Err(e) = self.send(&relay, &up, Sender::Guest, &me, Msg::Hello { name: self.my_name() }, &[]).await
        {
            self.set_health(generation, e.into());
        }
    }

    async fn on_message(&self, generation: u64, topic: &str, body: &str) {
        enum Who {
            Invite,
            Guest,
            Host,
            None,
        }
        let who = match &*self.inner.mode.lock() {
            Mode::Host(h) => {
                if h.invites.iter().any(|i| i.up.topic == topic) {
                    Who::Invite
                } else if h.guests.iter().any(|g| g.up.topic == topic) {
                    Who::Guest
                } else {
                    Who::None
                }
            }
            Mode::Guest(_) => Who::Host,
            _ => Who::None,
        };
        match who {
            Who::Invite => self.on_join(generation, topic, body).await,
            Who::Guest => self.on_guest(generation, topic, body).await,
            Who::Host => self.on_host(generation, topic, body).await,
            Who::None => {}
        }
    }

    // ---- hosting ----------------------------------------------------------

    /// Start hosting (or show the session already hosted).
    pub async fn start(&self) -> AppResult<ListenView> {
        let now = match &*self.inner.mode.lock() {
            Mode::Host(_) => 1,
            Mode::Guest(_) | Mode::Joining { .. } => 2,
            Mode::Idle => 0,
        };
        match now {
            1 => return Ok(self.view()),
            2 => return Err(AppError::Invalid("Leave the listening session you're in first.".into())),
            _ => {}
        }
        let machine = self
            .inner
            .deps
            .server
            .machine_id()
            .await
            .ok_or_else(|| AppError::Invalid("Connect to your Plex server first.".into()))?;
        let chosen = invite::relay_url(&self.inner.deps.settings.relay.read()).unwrap_or_else(|| self.inner.deps.default_relay.clone());
        let relay = self.inner.deps.relays.relay(&chosen);
        let session = seal::random16();
        let bcast_secret = seal::random32();
        let (topics_tx, topics_rx) = watch::channel(Vec::new());
        let mut h = Hosting {
            session,
            relay: relay.clone(),
            relay_in_link: (chosen != self.inner.deps.default_relay).then_some(chosen),
            srv: seal::server_hash(&session, &machine),
            invites: Vec::new(),
            guests: Vec::new(),
            bcast: Channel::new(&bcast_secret, "broadcast"),
            epoch: 0,
            bcast_secret,
            replay: ReplayGuard::default(),
            added: Vec::new(),
            topics: topics_tx,
            health: RelayHealth::Connecting,
            last: None,
            shown: None,
            songs: HashMap::new(),
        };
        self.add_invite(&mut h);
        let generation = {
            let mut mode = self.inner.mode.lock();
            if !matches!(&*mode, Mode::Idle) {
                return Err(AppError::Invalid("A listening session is already starting.".into()));
            }
            *mode = Mode::Host(h);
            self.bump()
        };
        tokio::spawn(self.clone().pump(generation, relay, topics_rx));
        tokio::spawn(self.clone().host_watch(generation));
        tracing::info!("listening session started");
        self.settings_changed().await;
        self.changed();
        Ok(self.view())
    }

    fn add_invite(&self, h: &mut Hosting) {
        let now = now_ms();
        h.invites.retain(|i| i.expires_ms > now);
        while h.invites.len() >= MAX_INVITES {
            h.invites.remove(0);
        }
        let hours = self.inner.deps.settings.invite_hours.load(Ordering::Relaxed).clamp(1, 24 * 7) as u64;
        let expires_ms = now + hours * 3_600_000;
        let token = Token { session: h.session, secret: seal::random16(), expires: (expires_ms / 1000) as u32, relay: h.relay_in_link.clone() };
        let (up, down) = invite_channels(&token);
        let link = token.link();
        h.invites.push(HostInvite { token, up, down, link, expires_ms, redeemed: None });
        h.refresh_topics();
    }

    /// The invite to send (host only): the open one, or a new one when
    /// `fresh` or when the last was used or ran out.
    pub fn invite(&self, fresh: bool) -> AppResult<ListenView> {
        {
            let mut mode = self.inner.mode.lock();
            let Mode::Host(h) = &mut *mode else {
                return Err(AppError::Invalid("Start a listening session first.".into()));
            };
            if fresh || h.current_invite(now_ms()).is_none() {
                self.add_invite(h);
            }
        }
        self.changed();
        Ok(self.view())
    }

    /// A guest answered an invite.
    async fn on_join(&self, generation: u64, topic: &str, body: &str) {
        let now = now_ms();
        let reply = {
            let mut mode = self.inner.mode.lock();
            let Mode::Host(h) = &mut *mode else { return };
            let Some(idx) = h.invites.iter().position(|i| i.up.topic == topic) else { return };
            let env = match seal::open(&h.invites[idx].up, body, now, None) {
                Ok(e) => e,
                Err(r) => return tracing::debug!(reason = ?r, "dropped an invite message"),
            };
            let (Sender::Guest, Msg::Join { n, pake, name }) = (env.r, env.m) else { return };
            if n.len() > 64 {
                return;
            }
            let full = h.guests.len() >= MAX_GUESTS;
            let inv = &mut h.invites[idx];
            let down = inv.down.clone();
            match &inv.redeemed {
                Some((nonce, answer)) if *nonce == n => Some((down, answer.clone(), false)),
                Some(_) => Some((down, Msg::Refused { n, why: Why::Used }, false)),
                None if inv.expires_ms <= now => Some((down, Msg::Refused { n, why: Why::Expired }, false)),
                None if full => Some((down, Msg::Refused { n, why: Why::Full }, false)),
                None => {
                    let Some(msg_a) = seal::unb64(&pake) else { return };
                    let Ok((msg_b, key)) = seal::pake_host(&inv.token.seed(), &msg_a) else { return };
                    let gid = hex(&seal::random16()[..8]);
                    let gsecret = seal::random32();
                    let welcome = Welcome {
                        id: gid.clone(),
                        g: seal::b64(&gsecret),
                        b: seal::b64(&h.bcast_secret),
                        e: h.epoch,
                        host: Person { id: HOST_ID.into(), name: self.my_name(), av: None },
                        srv: h.srv.clone(),
                    };
                    let Ok(sealed) = seal::seal_with(&key, &welcome_aad(&n), &serde_json::to_vec(&welcome).unwrap_or_default()) else { return };
                    let answer = Msg::Accepted { n: n.clone(), pake: seal::b64(&msg_b), sealed };
                    let inv = &mut h.invites[idx];
                    inv.redeemed = Some((n, answer.clone()));
                    let (up, gdown, tag) = guest_channels(&gsecret);
                    let name = clean_name(&name);
                    h.guests.push(HostGuest {
                        id: gid,
                        name: if name.is_empty() { "Guest".into() } else { name },
                        up,
                        down: gdown,
                        tag,
                        seen: Instant::now(),
                        hello_at: None,
                        cant: None,
                        offer: None,
                    });
                    h.refresh_topics();
                    Some((down, answer, true))
                }
            }
        };
        let Some((down, msg, joined)) = reply else { return };
        let relay = match &*self.inner.mode.lock() {
            Mode::Host(h) => h.relay.clone(),
            _ => return,
        };
        self.send_host(generation, &relay, &down, msg, &[]).await;
        if joined {
            tracing::info!("someone joined the listening session");
            self.changed();
        }
    }

    /// A message from a guest who's in.
    async fn on_guest(&self, generation: u64, topic: &str, body: &str) {
        enum Then {
            Nothing,
            State(Channel),
            Op(String, Op),
            Rekey(Vec<(Channel, Msg)>),
            Invite(String, String),
        }
        let now = now_ms();
        let (then, relay, renamed) = {
            let mut mode = self.inner.mode.lock();
            let Mode::Host(h) = &mut *mode else { return };
            let Some(i) = h.guests.iter().position(|g| g.up.topic == topic) else { return };
            let env = match seal::open(&h.guests[i].up, body, now, None) {
                Ok(e) => e,
                Err(r) => return tracing::debug!(reason = ?r, "dropped a guest message"),
            };
            if env.r != Sender::Guest || env.f != h.guests[i].id {
                return;
            }
            let gid = h.guests[i].id.clone();
            if let Err(r) = h.replay.accept(&gid, env.c) {
                return tracing::debug!(reason = ?r, "dropped a guest message");
            }
            h.guests[i].seen = Instant::now();
            let mut renamed = false;
            let then = match env.m {
                Msg::Hello { name } => {
                    let name = clean_name(&name);
                    if !name.is_empty() && name != h.guests[i].name {
                        h.guests[i].name = name;
                        renamed = true;
                    }
                    let g = &mut h.guests[i];
                    if g.hello_at.is_none_or(|t| t.elapsed() >= self.cfg().hello_every) {
                        g.hello_at = Some(Instant::now());
                        Then::State(g.down.clone())
                    } else {
                        Then::Nothing
                    }
                }
                Msg::Ping => Then::Nothing,
                Msg::CantHear { t } => {
                    let t = t.filter(|t| ok_song(t));
                    if h.guests[i].cant != t {
                        h.guests[i].cant = t;
                        renamed = true;
                    }
                    Then::Nothing
                }
                Msg::PlexUser { u } => {
                    let g = &mut h.guests[i];
                    if g.offer != Some(OfferState::Asking) {
                        Then::Nothing
                    } else {
                        renamed = true;
                        match u.map(|u| u.trim().to_string()).filter(|u| !u.is_empty() && u.len() <= 200 && !u.chars().any(char::is_control)) {
                            Some(u) => {
                                g.offer = Some(OfferState::Inviting);
                                Then::Invite(gid, u)
                            }
                            None => {
                                g.offer = Some(OfferState::NoInvites);
                                Then::Nothing
                            }
                        }
                    }
                }
                Msg::Cmd(op) => Then::Op(gid, op),
                Msg::Leave => {
                    h.guests.remove(i);
                    h.replay.forget(&gid);
                    h.refresh_topics();
                    renamed = true;
                    Then::Rekey(h.rekey())
                }
                _ => Then::Nothing,
            };
            (then, h.relay.clone(), renamed)
        };
        if renamed {
            self.changed();
            self.inner.poke.notify_one();
        }
        match then {
            Then::Nothing => {}
            Then::State(down) => {
                if let Some(st) = self.build_state().await {
                    self.send_host(generation, &relay, &down, Msg::State(st), &[]).await;
                }
            }
            Then::Op(gid, op) => {
                if let Err(msg) = self.run_op(&gid, op).await {
                    let down = match &*self.inner.mode.lock() {
                        Mode::Host(h) => h.guests.iter().find(|g| g.id == gid).map(|g| g.down.clone()),
                        _ => None,
                    };
                    if let Some(down) = down {
                        self.send_host(generation, &relay, &down, Msg::Denied { msg }, &[]).await;
                    }
                }
                self.inner.poke.notify_one();
            }
            Then::Rekey(msgs) => {
                tracing::info!("someone left the listening session");
                for (down, m) in msgs {
                    self.send_host(generation, &relay, &down, m, &[]).await;
                }
            }
            Then::Invite(gid, user) => {
                let outcome = self.inner.deps.offer.invite(&user).await;
                let state = match outcome {
                    OfferOutcome::Invited => OfferState::Invited,
                    OfferOutcome::AlreadyShared => OfferState::AlreadyShared,
                    OfferOutcome::Failed(m) => OfferState::Failed(m),
                };
                let tell = matches!(state, OfferState::Invited | OfferState::AlreadyShared);
                let down = match &mut *self.inner.mode.lock() {
                    Mode::Host(h) if self.current(generation) => h.guests.iter_mut().find(|g| g.id == gid).map(|g| {
                        g.offer = Some(state);
                        g.down.clone()
                    }),
                    _ => None,
                };
                if let Some(down) = down {
                    tracing::info!(invited = tell, "shared the server with someone in the session");
                    if tell {
                        self.send_host(generation, &relay, &down, Msg::Shared, &[]).await;
                    }
                    self.changed();
                }
            }
        }
    }

    /// "Let them listen from my server" (host only): ask the guest's Platter
    /// who they are on Plex; the invite follows its answer.
    pub async fn offer_server(&self, person_id: &str) -> AppResult<ListenView> {
        if !self.inner.deps.offer.offered().await {
            return Err(AppError::Invalid("Sharing your server is turned off in Settings, Sharing.".into()));
        }
        let ask = {
            let mut mode = self.inner.mode.lock();
            let Mode::Host(h) = &mut *mode else {
                return Err(AppError::Invalid("Only the host can share their server.".into()));
            };
            let Some(g) = h.guests.iter_mut().find(|g| g.id == person_id) else {
                return Err(AppError::NotFound("They're not in the session anymore.".into()));
            };
            // Asked already and waiting: once is enough.
            if matches!(g.offer, Some(OfferState::Asking | OfferState::Inviting)) {
                None
            } else {
                g.offer = Some(OfferState::Asking);
                Some((self.inner.generation.load(Ordering::SeqCst), h.relay.clone(), g.down.clone()))
            }
        };
        let Some((generation, relay, down)) = ask else { return Ok(self.view()) };
        self.send_host(generation, &relay, &down, Msg::Offer, &[]).await;
        self.changed();
        Ok(self.view())
    }

    /// Settings changed: whether the host offers its server.
    pub async fn settings_changed(&self) {
        let on = self.inner.deps.offer.offered().await;
        if self.inner.offered.swap(on, Ordering::Relaxed) != on {
            self.changed();
        }
    }

    /// The library here changed (a server was shared with this account and
    /// its music synced): look for the session's songs again, so a guest who
    /// couldn't hear one plays the host's copy now.
    pub async fn library_changed(&self) {
        let (generation, last) = match &mut *self.inner.mode.lock() {
            Mode::Guest(f) => {
                f.found.clear();
                (self.inner.generation.load(Ordering::SeqCst), f.last.clone())
            }
            _ => return,
        };
        if let Some(st) = last {
            self.follow(generation, st).await;
        }
    }

    /// Do what a guest asked. Err: why not, in words for them.
    /// A guest's request fits the rate: [`GUEST_OPS`] per [`GUEST_OPS_WINDOW`].
    fn guest_op_allowed(&self, gid: &str) -> bool {
        let now = std::time::Instant::now();
        let mut rate = self.inner.op_rate.lock();
        if rate.len() > 256 {
            rate.retain(|_, v| v.iter().any(|t| now.duration_since(*t) < GUEST_OPS_WINDOW));
        }
        let times = rate.entry(gid.to_string()).or_default();
        times.retain(|t| now.duration_since(*t) < GUEST_OPS_WINDOW);
        if times.len() >= GUEST_OPS {
            return false;
        }
        times.push(now);
        true
    }

    async fn run_op(&self, gid: &str, op: Op) -> Result<(), String> {
        let host = self.my_name();
        if gid != HOST_ID && !self.guest_op_allowed(gid) {
            return Err("Too many requests at once. Try again in a moment.".into());
        }
        if !self.can_control() && !matches!(op, Op::Add { .. }) {
            return Err(format!("{host} only lets others add songs."));
        }
        let p = &self.inner.deps.player;
        let failed = |_| "That didn't work on the host's side.".to_string();
        match op {
            Op::Play => p.resume().await.map_err(failed),
            Op::Pause => p.pause().await.map_err(failed),
            Op::Next => p.next().await.map_err(failed),
            Op::Previous => p.previous().await.map_err(failed),
            Op::Seek { ms } => p.seek(ms).await.map_err(failed),
            Op::Add { ids, songs } => {
                let refused = || "Those songs couldn't be added.".to_string();
                if ids.is_empty() || ids.len() > MAX_ADD || !ids.iter().all(|t| ok_song(t)) {
                    return Err(refused());
                }
                // A guest says who each song is in its own library; only ids
                // this library describes the same way go in. Someone trying
                // ids learns nothing: any mismatch, or an id that isn't
                // here, gets the same answer.
                if gid != HOST_ID {
                    if songs.len() != ids.len() {
                        return Err(refused());
                    }
                    let here = self.inner.deps.library.songs(ids.clone()).await;
                    if !ids.iter().zip(&songs).all(|(id, s)| here.get(id).is_some_and(|h| same_song(h, s))) {
                        return Err(refused());
                    }
                }
                p.queue_add(ids.clone()).await.map_err(|_| "Those songs couldn't be added.".to_string())?;
                if let Mode::Host(h) = &mut *self.inner.mode.lock() {
                    h.added.extend(ids.into_iter().map(|k| (k, gid.to_string())));
                }
                Ok(())
            }
            Op::Move { from, to } => {
                let now = p.now().await.map_err(failed)?;
                let item = now.up_next_items.get(from).cloned().ok_or_else(|| "That song isn't in the queue anymore.".to_string())?;
                p.queue_move(item, to).await.map_err(failed)
            }
        }
    }

    /// The state to send, from the player now. None when nobody's listening.
    async fn build_state(&self) -> Option<State> {
        if !matches!(&*self.inner.mode.lock(), Mode::Host(h) if !h.guests.is_empty()) {
            return None;
        }
        let now = self.inner.deps.player.now().await.unwrap_or_default();
        let ctl = self.can_control();
        let me = self.my_name();
        let hide = self.inner.deps.settings.hide_unreleased.load(Ordering::Relaxed);
        // Who the songs are: looked up once each, outside the lock.
        let ids: Vec<String> = now.track_id.iter().chain(now.up_next.iter().take(UP_NEXT)).cloned().collect();
        let unknown: Vec<String> = match &*self.inner.mode.lock() {
            Mode::Host(h) => ids.iter().filter(|k| !h.songs.contains_key(*k)).cloned().collect(),
            _ => return None,
        };
        let looked = if unknown.is_empty() { HashMap::new() } else { self.inner.deps.library.songs(unknown.clone()).await };
        let mut mode = self.inner.mode.lock();
        let Mode::Host(h) = &mut *mode else { return None };
        if h.songs.len() + unknown.len() > 2000 {
            h.songs.clear();
        }
        for k in unknown {
            let s = looked.get(&k).cloned();
            h.songs.insert(k, s);
        }
        let songs = &h.songs;
        let song = |k: &str| songs.get(k).cloned().flatten().map(|s| matching::shown(s, hide));
        let mut pool = std::mem::take(&mut h.added);
        let mut kept = Vec::new();
        let mut take = |k: &str| {
            let i = pool.iter().position(|(t, _)| t == k)?;
            let e = pool.remove(i);
            let by = e.1.clone();
            kept.push(e);
            Some(by)
        };
        let tby = now.track_id.as_deref().and_then(&mut take);
        let q: Vec<Queued> = now.up_next.iter().take(UP_NEXT).map(|k| Queued { k: k.clone(), by: take(k), s: song(k) }).collect();
        let ts = now.track_id.as_deref().and_then(song);
        h.added = kept;
        let mut ppl = vec![Person { id: HOST_ID.into(), name: me, av: None }];
        ppl.extend(h.guests.iter().map(|g| Person { id: g.id.clone(), name: g.name.clone(), av: None }));
        let mut st = State { srv: h.srv.clone(), t: now.track_id, ts, tby, p: now.position_ms, d: now.duration_ms, pl: now.playing, q, ctl, ppl };
        fit(&mut st, h.guests.len());
        h.shown = Some(st.clone());
        Some(st)
    }

    /// Watch the player and the guests; send the state when it changes.
    async fn host_watch(self, generation: u64) {
        let cfg = self.cfg();
        let mut looked = Instant::now();
        loop {
            // "Let them listen from my server" may have been switched in Settings.
            if looked.elapsed() >= Duration::from_secs(5) {
                looked = Instant::now();
                self.settings_changed().await;
            }
            tokio::select! {
                _ = tokio::time::sleep(cfg.watch) => {}
                _ = self.inner.poke.notified() => {}
            }
            if !self.current(generation) {
                return;
            }
            // Expired invites and quiet guests.
            let (rekey, relay, changed) = {
                let mut mode = self.inner.mode.lock();
                let Mode::Host(h) = &mut *mode else { return };
                let now = now_ms();
                let before = (h.invites.len(), h.guests.len());
                h.invites.retain(|i| i.expires_ms > now);
                h.guests.retain(|g| g.seen.elapsed() < cfg.guest_timeout);
                let gone = h.guests.len() != before.1;
                let changed = gone || h.invites.len() != before.0;
                if changed {
                    h.refresh_topics();
                }
                (if gone { h.rekey() } else { Vec::new() }, h.relay.clone(), changed)
            };
            for (down, m) in rekey {
                self.send_host(generation, &relay, &down, m, &[]).await;
            }
            if changed {
                self.changed();
            }
            let Some(st) = self.build_state().await else { continue };
            let send = {
                let mode = self.inner.mode.lock();
                let Mode::Host(h) = &*mode else { return };
                match &h.last {
                    None => true,
                    Some(last) => {
                        let mut a = last.sig.clone();
                        let mut b = st.clone();
                        let elapsed = last.at.elapsed().as_millis() as u64;
                        let expected = if a.pl { a.p + elapsed } else { a.p };
                        let jumped = a.t == b.t && b.p.abs_diff(expected) > SEEK_JUMP_MS;
                        (a.p, b.p) = (0, 0);
                        a != b || jumped || last.at.elapsed() >= cfg.heartbeat
                    }
                }
            };
            if !send {
                continue;
            }
            let (bcast, tags) = {
                let mut mode = self.inner.mode.lock();
                let Mode::Host(h) = &mut *mode else { return };
                h.last = Some(Sent { sig: st.clone(), at: Instant::now() });
                (h.bcast.clone(), h.guests.iter().map(|g| g.tag).collect::<Vec<_>>())
            };
            let had = self.view().queue;
            self.send_host(generation, &relay, &bcast, Msg::State(st), &tags).await;
            if self.view().queue != had {
                self.changed();
            }
        }
    }

    /// Remove a guest (host only): they're told, and lose the keys.
    pub async fn remove(&self, person_id: &str) -> AppResult<ListenView> {
        let (generation, relay, told, rekey) = {
            let mut mode = self.inner.mode.lock();
            let Mode::Host(h) = &mut *mode else {
                return Err(AppError::Invalid("Only the host can remove people.".into()));
            };
            let Some(i) = h.guests.iter().position(|g| g.id == person_id) else {
                return Err(AppError::NotFound("They're not in the session anymore.".into()));
            };
            let g = h.guests.remove(i);
            h.replay.forget(&g.id);
            h.added.retain(|(_, by)| by != &g.id);
            h.refresh_topics();
            (self.inner.generation.load(Ordering::SeqCst), h.relay.clone(), g.down, h.rekey())
        };
        self.send_host(generation, &relay, &told, Msg::Removed, &[]).await;
        for (down, m) in rekey {
            self.send_host(generation, &relay, &down, m, &[]).await;
        }
        tracing::info!("removed someone from the listening session");
        self.changed();
        self.inner.poke.notify_one();
        Ok(self.view())
    }

    /// End the session (host) or leave it (guest).
    pub async fn stop(&self) -> AppResult<()> {
        let old = std::mem::replace(&mut *self.inner.mode.lock(), Mode::Idle);
        self.bump();
        self.inner.poke.notify_one();
        match old {
            Mode::Idle => return Ok(()),
            Mode::Joining { .. } => {}
            Mode::Host(h) => {
                if !h.guests.is_empty() {
                    let tags: Vec<[u8; 32]> = h.guests.iter().map(|g| g.tag).collect();
                    let _ = self.send(&h.relay, &h.bcast, Sender::Host, HOST_ID, Msg::Ended, &tags).await;
                }
                tracing::info!("listening session ended");
            }
            Mode::Guest(f) => {
                if matches!(f.status, SessionStatus::Live | SessionStatus::Connecting | SessionStatus::HostGone) {
                    let _ = self.send(&f.relay, &f.up, Sender::Guest, &f.me, Msg::Leave, &[]).await;
                }
                tracing::info!("left the listening session");
            }
        }
        self.changed();
        Ok(())
    }

    // ---- following --------------------------------------------------------

    /// Join a session from its invite link.
    pub async fn join(&self, link: &str) -> AppResult<ListenView> {
        let token = invite::parse(link).ok_or_else(|| AppError::Invalid("That isn't a listening session link.".into()))?;
        if u64::from(token.expires) * 1000 <= now_ms() {
            return Err(AppError::Invalid("That invite has expired. Ask for a new one.".into()));
        }
        if !invite::link_relay_ok(token.relay.as_deref(), &self.inner.deps.settings.relay.read()) {
            return Err(AppError::Invalid(invite::UNSAFE_RELAY.into()));
        }
        let now = match &*self.inner.mode.lock() {
            Mode::Host(_) => Err("End your own listening session first."),
            Mode::Joining { .. } => Err("Already joining a listening session."),
            Mode::Guest(f) if f.session == token.session && f.status == SessionStatus::Live => Ok(None),
            Mode::Guest(_) => Ok(Some(true)),
            Mode::Idle => Ok(Some(false)),
        };
        let leave_first = match now {
            Err(msg) => return Err(AppError::Invalid(msg.into())),
            Ok(None) => return Ok(self.view()),
            Ok(Some(b)) => b,
        };
        if leave_first {
            self.stop().await?;
        }
        let machine = self
            .inner
            .deps
            .server
            .machine_id()
            .await
            .ok_or_else(|| AppError::Invalid("Connect to your Plex server first.".into()))?;
        let generation = {
            let mut mode = self.inner.mode.lock();
            if !matches!(&*mode, Mode::Idle) {
                return Err(AppError::Invalid("Already in a listening session.".into()));
            }
            *mode = Mode::Joining { session: token.session_hex() };
            self.bump()
        };
        self.changed();
        let base = token.relay.clone().unwrap_or_else(|| self.inner.deps.default_relay.clone());
        let relay = self.inner.deps.relays.relay(&base);
        let result = self.redeem(&relay, &token, &machine).await;
        let fail = |e: AppError| {
            if self.current(generation) {
                *self.inner.mode.lock() = Mode::Idle;
                self.changed();
            }
            Err(e)
        };
        let f = match result {
            Ok(f) => f,
            Err(e) => return fail(e),
        };
        let topics_rx = f.topics.subscribe();
        {
            let mut mode = self.inner.mode.lock();
            if !self.current(generation) {
                return Err(AppError::Invalid("Joining was cancelled.".into()));
            }
            *mode = Mode::Guest(f);
        }
        tokio::spawn(self.clone().pump(generation, relay, topics_rx));
        tokio::spawn(self.clone().guest_tick(generation));
        tracing::info!("joined a listening session");
        self.changed();
        Ok(self.view())
    }

    /// Answer the invite and wait for the host to let us in.
    async fn redeem(&self, relay: &Arc<dyn Relay>, token: &Token, machine: &str) -> AppResult<Following> {
        let cfg = self.cfg();
        let (up, down) = invite_channels(token);
        let mut rx = relay.subscribe(std::slice::from_ref(&down.topic), None).await.map_err(relay_err)?;
        let deadline = Instant::now() + cfg.answer_timeout;
        let no_answer = || AppError::NotFound("No answer from the host. The invite may have been used or run out, or the session ended.".into());
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(RelayEvent::Open)) => break,
                Ok(Some(_)) => continue,
                Ok(None) => return Err(relay_err(RelayError::Unreachable)),
                Err(_) => return Err(relay_err(RelayError::Unreachable)),
            }
        }
        let (pake, msg_a) = seal::pake_guest_start(&token.seed());
        let n = hex(&seal::random16()[..8]);
        let join = Msg::Join { n: n.clone(), pake: seal::b64(&msg_a), name: self.my_name() };
        self.send(relay, &up, Sender::Guest, &n, join, &[]).await.map_err(relay_err)?;
        let welcome = loop {
            let ev = match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(ev)) => ev,
                Ok(None) => return Err(relay_err(RelayError::Unreachable)),
                Err(_) => return Err(no_answer()),
            };
            let RelayEvent::Message { topic, body, .. } = ev else { continue };
            if topic != down.topic {
                continue;
            }
            let Ok(env) = seal::open(&down, &body, now_ms(), None) else { continue };
            if env.r != Sender::Host {
                continue;
            }
            match env.m {
                Msg::Accepted { n: an, pake: msg_b, sealed } if an == n => {
                    let msg_b = seal::unb64(&msg_b).ok_or_else(no_answer)?;
                    let key = seal::pake_guest_finish(pake, &msg_b)?;
                    let plain = seal::open_with(&key, &welcome_aad(&n), &sealed)?;
                    break serde_json::from_slice::<Welcome>(&plain).map_err(|_| AppError::Decode("unreadable welcome".into()))?;
                }
                Msg::Refused { n: an, why } if an == n => {
                    return Err(AppError::Invalid(
                        match why {
                            Why::Used => "That invite was already used. Ask for a new one.",
                            Why::Expired => "That invite has expired. Ask for a new one.",
                            Why::Full => "That listening session is full.",
                        }
                        .into(),
                    ));
                }
                _ => continue,
            }
        };
        drop(rx);
        let bad = || AppError::Decode("unreadable welcome".into());
        let gsecret = seal::key32(&welcome.g).ok_or_else(bad)?;
        let bsecret = seal::key32(&welcome.b).ok_or_else(bad)?;
        if !ok_id(&welcome.id) {
            return Err(bad());
        }
        let (gup, gdown, tag) = guest_channels(&gsecret);
        let host = Person { id: HOST_ID.into(), name: clean_name(&welcome.host.name), av: None };
        // Another server than the host's is fine: each song plays from the
        // host's copy when this library has it, else the same song here, else
        // waits for the next one.
        if seal::server_hash(&token.session, machine) != welcome.srv {
            tracing::info!("listening along from another server");
        }
        let bcast = Channel::new(&bsecret, "broadcast");
        let topics = watch::channel(vec![bcast.topic.clone(), gdown.topic.clone()]).0;
        Ok(Following {
            session: token.session,
            relay: relay.clone(),
            me: welcome.id,
            host,
            up: gup,
            down: gdown,
            tag,
            bcast,
            epoch: welcome.e,
            status: SessionStatus::Connecting,
            health: RelayHealth::Ok,
            replay: ReplayGuard::default(),
            last: None,
            heard: Instant::now(),
            topics,
            found: HashMap::new(),
            missing: None,
            other_version: false,
            told: None,
        })
    }

    /// A message from the host.
    async fn on_host(&self, generation: u64, topic: &str, body: &str) {
        let now = now_ms();
        let mut follow = None;
        let mut stopped = false;
        let mut denied = None;
        // The host offers its server (true) or says it shared it (false).
        let mut offer = None;
        let changed = {
            let mut mode = self.inner.mode.lock();
            let Mode::Guest(f) = &mut *mode else { return };
            let (env, chan) = if topic == f.bcast.topic {
                (seal::open(&f.bcast, body, now, Some(&f.tag)), format!("b{}", f.epoch))
            } else if topic == f.down.topic {
                (seal::open(&f.down, body, now, None), "d".to_string())
            } else {
                return;
            };
            let env = match env {
                Ok(e) if e.r == Sender::Host && e.f == HOST_ID => e,
                Ok(_) => return,
                Err(r) => return tracing::debug!(reason = ?r, "dropped a host message"),
            };
            if let Err(r) = f.replay.accept(&chan, env.c) {
                return tracing::debug!(reason = ?r, "dropped a host message");
            }
            f.heard = Instant::now();
            let before = (f.status, f.last.as_ref().map(|s| (s.ppl.clone(), s.ctl, s.q.clone(), s.tby.clone())));
            match env.m {
                Msg::State(st) => {
                    if f.status == SessionStatus::Connecting || f.status == SessionStatus::HostGone {
                        f.status = SessionStatus::Live;
                    }
                    follow = Some(st.clone());
                    f.last = Some(st);
                }
                Msg::Ended => {
                    f.status = SessionStatus::Ended;
                    stopped = true;
                }
                Msg::Removed => {
                    f.status = SessionStatus::Removed;
                    stopped = true;
                }
                Msg::Rekey { b, e } if e > f.epoch => {
                    if let Some(secret) = seal::key32(&b) {
                        f.bcast = Channel::new(&secret, "broadcast");
                        f.epoch = e;
                        let t = f.topic_list();
                        f.topics.send_replace(t);
                    }
                }
                Msg::Denied { msg } => denied = Some(msg),
                Msg::Offer => offer = Some(true),
                Msg::Shared => offer = Some(false),
                _ => {}
            }
            before != (f.status, f.last.as_ref().map(|s| (s.ppl.clone(), s.ctl, s.q.clone(), s.tby.clone())))
        };
        if stopped && self.current(generation) {
            self.bump();
            tracing::info!("the listening session stopped");
        }
        if changed {
            self.changed();
        }
        if let Some(msg) = denied {
            self.notice(&msg);
        }
        match offer {
            Some(true) => self.answer_offer(generation).await,
            Some(false) => {
                let host = match &*self.inner.mode.lock() {
                    Mode::Guest(f) => f.host.name.clone(),
                    _ => return,
                };
                // Looking for the invite can take a moment; the session goes on.
                let me = self.clone();
                tokio::spawn(async move { me.inner.deps.offer.shared(&host).await });
            }
            None => {}
        }
        if let Some(st) = follow {
            self.follow(generation, st).await;
        }
    }

    /// The host offers its server: tell it who we are on Plex, only when
    /// this computer takes invites inside Platter (else None, and the host
    /// is told to share from Settings).
    async fn answer_offer(&self, generation: u64) {
        let user = self.inner.deps.offer.me().await.map(|u| u.trim().chars().filter(|c| !c.is_control()).take(200).collect::<String>()).filter(|u| !u.is_empty());
        let target = match &*self.inner.mode.lock() {
            Mode::Guest(f) if self.current(generation) => Some((f.relay.clone(), f.up.clone(), f.me.clone())),
            _ => None,
        };
        if let Some((relay, up, me)) = target
            && let Err(e) = self.send(&relay, &up, Sender::Guest, &me, Msg::PlexUser { u: user }, &[]).await
        {
            self.set_health(generation, e.into());
        }
    }

    /// Match the host's state.
    async fn follow(&self, generation: u64, st: State) {
        let _one = self.inner.follow_lock.lock().await;
        if !self.current(generation) {
            return;
        }
        let player = &self.inner.deps.player;
        let Ok(local) = player.now().await else { return };
        let here = self.localize(&st).await;
        let mut cant = None;
        let steps = match &here {
            Some(mapped) => plan(&local, mapped, LATENCY_MS),
            // Nothing to play here: wait quietly for the next song.
            None => {
                cant = st.t.clone();
                if local.playing { vec![Step::Pause] } else { Vec::new() }
            }
        };
        let loads = steps.iter().any(|s| matches!(s, Step::Load { .. }));
        if !steps.is_empty()
            && let Err(e) = apply(player.as_ref(), steps).await
        {
            tracing::debug!(kind = e.kind(), "couldn't follow the host");
            if loads {
                // The copy wouldn't load: the same as having none.
                let _ = player.pause().await;
                cant = st.t.clone();
                self.set_here(generation, Some(missing_song(st.ts.as_ref())), false);
            } else {
                self.notice("A song in the session couldn't play here.");
            }
        }
        self.tell_host(generation, cant).await;
    }

    /// The host's state as this library plays it: the song playing as the
    /// host's copy or one here (at the same share of the song when its
    /// length differs), and the up next songs that have a copy here, up to
    /// the first that has none. None: the song playing has no copy here.
    async fn localize(&self, st: &State) -> Option<State> {
        let generation = self.inner.generation.load(Ordering::SeqCst);
        let Some(t) = st.t.clone() else {
            self.set_here(generation, None, false);
            return Some(st.clone());
        };
        let Some(found) = self.copy_here(&t, st.ts.as_ref()).await else {
            self.set_here(generation, Some(missing_song(st.ts.as_ref())), false);
            return None;
        };
        let other = found.id != t && found.duration_ms > 0 && st.d > 0 && found.duration_ms.abs_diff(st.d) > matching::SLACK_MS;
        let mut mapped = st.clone();
        mapped.t = Some(found.id.clone());
        if other {
            mapped.p = matching::map_position(st.p, st.d, found.duration_ms);
            mapped.d = found.duration_ms;
        }
        mapped.q = Vec::new();
        for q in &st.q {
            match self.copy_here(&q.k, q.s.as_ref()).await {
                Some(f) => mapped.q.push(Queued { k: f.id, ..q.clone() }),
                None => break,
            }
        }
        self.set_here(generation, None, other);
        Some(mapped)
    }

    /// The copy here of the host's song `id`: the host's own when this
    /// library has it, else the same song by `song`. Remembered per song.
    async fn copy_here(&self, id: &str, song: Option<&Song>) -> Option<Found> {
        if let Mode::Guest(f) = &*self.inner.mode.lock()
            && let Some(hit) = f.found.get(id)
        {
            return hit.clone();
        }
        let lib = &self.inner.deps.library;
        let hit = if lib.has(id).await {
            Some(Found { id: id.to_string(), duration_ms: song.map(|s| s.d).unwrap_or(0) })
        } else {
            match song {
                Some(s) if !s.u => lib.find(s).await,
                _ => None,
            }
        };
        // Without who the song is (a long queue's tail), ask again later.
        if (hit.is_some() || song.is_some())
            && let Mode::Guest(f) = &mut *self.inner.mode.lock()
        {
            if f.found.len() >= 2000 {
                f.found.clear();
            }
            f.found.insert(id.to_string(), hit.clone());
        }
        hit
    }

    /// Show whether the song playing has a copy here.
    fn set_here(&self, generation: u64, missing: Option<MissingSong>, other_version: bool) {
        let differs = match &mut *self.inner.mode.lock() {
            Mode::Guest(f) if self.current(generation) => {
                let before = (f.missing.clone(), f.other_version);
                f.missing = missing;
                f.other_version = other_version;
                before != (f.missing.clone(), f.other_version)
            }
            _ => false,
        };
        if differs {
            self.changed();
        }
    }

    /// Tell the host which song we can't hear (None: we hear it again), once.
    async fn tell_host(&self, generation: u64, cant: Option<String>) {
        let target = match &mut *self.inner.mode.lock() {
            Mode::Guest(f) if self.current(generation) && f.told != cant => {
                f.told = cant.clone();
                Some((f.relay.clone(), f.up.clone(), f.me.clone()))
            }
            _ => None,
        };
        if let Some((relay, up, me)) = target
            && let Err(e) = self.send(&relay, &up, Sender::Guest, &me, Msg::CantHear { t: cant }, &[]).await
        {
            self.set_health(generation, e.into());
        }
    }

    /// The guest's clock: is the host still there, and say we are.
    async fn guest_tick(self, generation: u64) {
        let cfg = self.cfg();
        let mut pinged = Instant::now();
        loop {
            tokio::time::sleep(cfg.watch).await;
            if !self.current(generation) {
                return;
            }
            let (gone, ping) = {
                let mut mode = self.inner.mode.lock();
                let Mode::Guest(f) = &mut *mode else { return };
                let gone = matches!(f.status, SessionStatus::Live | SessionStatus::Connecting) && f.heard.elapsed() >= cfg.heartbeat + cfg.grace;
                if gone {
                    f.status = SessionStatus::HostGone;
                }
                let ping = (pinged.elapsed() >= cfg.guest_ping).then(|| (f.relay.clone(), f.up.clone(), f.me.clone()));
                (gone, ping)
            };
            if gone {
                tracing::info!("the host stopped answering");
                self.changed();
            }
            if let Some((relay, up, me)) = ping {
                pinged = Instant::now();
                if let Err(e) = self.send(&relay, &up, Sender::Guest, &me, Msg::Ping, &[]).await {
                    self.set_health(generation, e.into());
                }
            }
        }
    }

    /// Ask the host to do something (guest), or do it (host).
    pub async fn control(&self, op: Op) -> AppResult<()> {
        if let Op::Add { ids, .. } = &op
            && (ids.is_empty() || ids.len() > MAX_ADD || !ids.iter().all(|t| ok_song(t)))
        {
            return Err(AppError::Invalid("Bad song ids.".into()));
        }
        let target = match &*self.inner.mode.lock() {
            Mode::Guest(f) if f.status == SessionStatus::Live => Some((f.relay.clone(), f.up.clone(), f.me.clone(), f.last.as_ref().is_some_and(|s| s.ctl), f.host.name.clone())),
            Mode::Host(_) => None,
            _ => return Err(AppError::Invalid("You're not in a listening session.".into())),
        };
        let Some((relay, up, me, ctl, host)) = target else {
            return self.run_op(HOST_ID, op).await.map_err(AppError::Invalid);
        };
        if !ctl && !matches!(op, Op::Add { .. }) {
            return Err(AppError::Invalid(format!("{host} only lets others add songs.")));
        }
        // Say who each added song is here, so the host can tell it's one it
        // has too.
        let op = match op {
            Op::Add { ids, .. } => {
                let known = self.inner.deps.library.songs(ids.clone()).await;
                let songs = ids.iter().map(|id| known.get(id).cloned().unwrap_or_default()).collect();
                Op::Add { ids, songs }
            }
            other => other,
        };
        self.send(&relay, &up, Sender::Guest, &me, Msg::Cmd(op), &[]).await.map_err(relay_err)
    }

    /// Add songs to the shared queue (a guest's go through the host).
    pub async fn add(&self, track_ids: Vec<String>) -> AppResult<()> {
        self.control(Op::Add { ids: track_ids, songs: Vec::new() }).await
    }
}

fn missing_song(s: Option<&Song>) -> MissingSong {
    MissingSong {
        title: s.and_then(|s| s.n.clone()),
        artist: s.and_then(|s| s.a.clone()),
        unreleased: s.is_some_and(|s| s.u),
    }
}

fn queue_view(st: &State) -> Vec<QueuedView> {
    st.q.iter().map(|q| QueuedView { track_id: q.k.clone(), by: q.by.clone() }).collect()
}

/// Fit the sealed message in the relay's size limit: first drop who the
/// songs are from the end of the queue (the next song keeps it), then songs
/// from the end.
fn fit(st: &mut State, guests: usize) {
    let size = |st: &State| {
        let plain = serde_json::to_vec(&Msg::State(st.clone())).map(|v| v.len()).unwrap_or(usize::MAX) + 96;
        2 + (12 + plain + 16).div_ceil(3) * 4 + 1 + (guests * 16).div_ceil(3) * 4
    };
    let mut known = st.q.iter().filter(|q| q.s.is_some()).count();
    while size(st) > seal::MAX_BODY && known > 1 {
        known = known.div_ceil(2).min(known - 1);
        for q in st.q.iter_mut().filter(|q| q.s.is_some()).skip(known) {
            q.s = None;
        }
    }
    while size(st) > seal::MAX_BODY && !st.q.is_empty() {
        let keep = st.q.len() / 2;
        st.q.truncate(keep);
    }
}
