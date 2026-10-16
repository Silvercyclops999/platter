//! Sharing your music through Plex: "Share my music" (Settings, Sharing),
//! the people with access, and "Let them listen from my server" in Listen
//! along. There's no Platter server: every share is a Plex share made on
//! plex.tv (`plex::sharing`), of music libraries only, and Plex's own rules
//! decide what friends can play and from where.
//!
//! Unreleased music stays home. Before a share, Platter counts the
//! unreleased albums in the chosen libraries (the one release rule:
//! `platform::discord::commands::album_unreleased`, from each file's
//! provenance and the album). With "Hide my unreleased music from people I
//! share with" on, those albums get the Plex label [`HIDE_LABEL`] on the
//! server and the share leaves that label out (`label!=`), so friends never
//! see them. Label restrictions need the host's Plex Pass; without it the
//! share isn't made and the host picks: share the other libraries, or not
//! at all. If labelling fails nothing is shared.
//!
//! The other side: invites this account received show in Platter (a
//! notification and a card), and accepting one adds the friend's server.
//! plex.tv lists it, the connection race picks its address, and its music
//! libraries join the library here, so in a listening session the host's
//! own copy plays ([`ServerOffer::shared`] and
//! `ListenAlong::library_changed`).
//!
//! [`links`]: a link or QR code that asks someone to share with you, its
//! request carried through the sealed relay.

pub mod links;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use super::Notifier;
use super::listen::{OfferOutcome, ServerOffer};
use crate::db::DbHandle;
use crate::db::store::{self, MAIN_SRC};
use crate::error::{AppError, AppResult};
use crate::libraries::{Section, ServerEntry};
use crate::plex::auth::PlexTv;
use crate::plex::sharing::{ReceivedInvite, ShareTerms, SharedWith, TvSection, is_already_shared};
use crate::plex::state::EventSink;
use crate::plex::token::Token;

/// The invites this account received changed (a list of [`ReceivedInvite`]).
pub const EVENT_INVITES: &str = "sharing://invites";
/// Someone asked for your music through a share link; payload [`Notice`].
pub const EVENT_NOTICE: &str = "sharing://notice";
/// The Plex label on unreleased albums, left out of every share.
pub const HIDE_LABEL: &str = "platter-unreleased";
/// Where the albums Platter labelled are remembered (so later shares skip them).
const LABELLED_KEY: &str = "sharing.labelled";

/// The settings sharing reads (all synced).
#[derive(Clone)]
pub struct ShareSettings {
    /// Offer "Let them listen from my server" (host only).
    pub offer: Arc<AtomicBool>,
    /// Libraries I share by default: section ids on my server, music only.
    pub default_libraries: Arc<RwLock<Vec<String>>>,
    /// Hide my unreleased music from people I share with.
    pub hide_unreleased: Arc<AtomicBool>,
    /// Allow downloads for people I share with.
    pub allow_downloads: Arc<AtomicBool>,
    /// Accept invites inside Platter.
    pub accept_in_app: Arc<AtomicBool>,
}

impl Default for ShareSettings {
    fn default() -> Self {
        Self {
            offer: Arc::new(AtomicBool::new(true)),
            default_libraries: Arc::default(),
            hide_unreleased: Arc::new(AtomicBool::new(true)),
            allow_downloads: Arc::new(AtomicBool::new(false)),
            accept_in_app: Arc::new(AtomicBool::new(true)),
        }
    }
}

/// The server this computer plays from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MyServer {
    pub machine_id: String,
    pub name: String,
    pub owned: bool,
}

/// Plex, as sharing needs it. The app's is [`PlexHost`]; tests fake the
/// server side and point the plex.tv client at a fake plex.tv.
#[async_trait]
pub trait Host: Send + Sync {
    fn tv(&self) -> &PlexTv;
    async fn token(&self) -> Option<Token>;
    async fn my_server(&self) -> Option<MyServer>;
    /// The main library's section id on my server.
    async fn main_section(&self) -> Option<String>;
    /// This account's Plex username.
    async fn username(&self) -> Option<String>;
    async fn add_label(&self, section: &str, album_id: &str, label: &str) -> AppResult<bool>;
    /// The account's servers as known now, and after asking plex.tv again.
    async fn servers(&self) -> Vec<ServerEntry>;
    async fn refresh_servers(&self) -> AppResult<Vec<ServerEntry>>;
    async fn music_sections(&self, machine_id: &str) -> AppResult<Vec<Section>>;
    /// Include these libraries (`<machine id>/<section id>`) and sync them.
    async fn include(&self, entries: Vec<String>) -> AppResult<()>;
}

// ---- what the UI sees ---------------------------------------------------------

/// A music library on my server, for choosing what to share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryChoice {
    pub id: String,
    pub title: String,
    /// Unreleased albums in it. None: Platter hasn't read this library, so
    /// it can't tell (include it in Settings, Library first).
    pub unreleased: Option<u32>,
    /// In "Libraries I share by default".
    pub default: bool,
}

/// `sharing_overview`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SharingOverview {
    pub server_name: Option<String>,
    /// This account owns the server it plays from (only owners share).
    pub owner: bool,
    /// None when plex.tv couldn't say.
    pub plex_pass: Option<bool>,
    /// Music libraries only.
    pub libraries: Vec<LibraryChoice>,
}

/// What a share gives.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareOptions {
    pub libraries: Vec<String>,
    pub hide_unreleased: bool,
    pub allow_downloads: bool,
}

/// How a share went (`sharing_share`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum ShareStatus {
    /// Plex sent the invite; `hidden` albums are kept out.
    #[serde(rename_all = "camelCase")]
    Invited { hidden: u32 },
    /// They can already see this server.
    AlreadyShared,
    /// The chosen libraries hold unreleased albums, hiding them needs Plex
    /// Pass, and this account has none. Nothing was shared. `others`: the
    /// chosen libraries without unreleased music, to share instead.
    #[serde(rename_all = "camelCase")]
    NeedsPlexPass { with_unreleased: Vec<String>, others: Vec<String> },
    /// Plex doesn't know that username or email.
    UnknownUser { message: String },
    Refused { message: String },
    /// plex.tv (or the server) isn't answering.
    PlexDown { message: String },
}

/// Someone with access to my server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersonWithAccess {
    pub id: String,
    pub name: String,
    /// The library names they see.
    pub libraries: Vec<String>,
    pub allow_downloads: bool,
    /// The share leaves out albums with [`HIDE_LABEL`].
    pub unreleased_hidden: bool,
    /// Invited, not accepted yet.
    pub pending: bool,
    /// Unix seconds.
    pub last_seen_at: Option<i64>,
}

/// `sharing_accept`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Accepted {
    pub servers: Vec<String>,
    /// Music libraries added to the library here.
    pub libraries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub message: String,
}

// ---- the service ----------------------------------------------------------------

/// Called after an accept with the libraries added (the app waits for their
/// sync, then tells listen along).
pub type OnAccepted = Arc<dyn Fn(Vec<String>) + Send + Sync>;

struct Inner {
    host: Arc<dyn Host>,
    db: DbHandle,
    settings: ShareSettings,
    events: Arc<dyn EventSink>,
    notifier: Arc<dyn Notifier>,
    pending: Mutex<Vec<ReceivedInvite>>,
    /// Invites already announced.
    announced: Mutex<HashSet<String>>,
    on_accepted: Mutex<Option<OnAccepted>>,
    links: links::Links,
}

/// Cheap to clone.
#[derive(Clone)]
pub struct Sharing {
    inner: Arc<Inner>,
}

fn plex_down(e: &AppError) -> bool {
    matches!(e, AppError::Network(_) | AppError::Server(_))
}

impl Sharing {
    pub fn new(host: Arc<dyn Host>, db: DbHandle, settings: ShareSettings, events: Arc<dyn EventSink>, notifier: Arc<dyn Notifier>, links: links::LinkDeps) -> Self {
        Self {
            inner: Arc::new(Inner {
                host,
                db,
                settings,
                events,
                notifier,
                pending: Mutex::default(),
                announced: Mutex::default(),
                on_accepted: Mutex::default(),
                links: links::Links::new(links),
            }),
        }
    }

    pub fn set_on_accepted(&self, f: OnAccepted) {
        *self.inner.on_accepted.lock() = Some(f);
    }

    pub fn settings(&self) -> &ShareSettings {
        &self.inner.settings
    }

    async fn token(&self) -> AppResult<Token> {
        self.inner.host.token().await.ok_or_else(|| AppError::Auth("Sign in to Plex first.".into()))
    }

    /// My server, when this account owns it.
    async fn owned_server(&self) -> AppResult<MyServer> {
        match self.inner.host.my_server().await {
            Some(s) if s.owned => Ok(s),
            Some(_) => Err(AppError::Invalid("Only the server's owner can share it. This server is shared with you.".into())),
            None => Err(AppError::Invalid("Connect to your Plex server first.".into())),
        }
    }

    /// My server's music libraries as plex.tv numbers them.
    async fn music(&self, token: &Token, machine: &str) -> AppResult<Vec<TvSection>> {
        Ok(self.inner.host.tv().server_sections(token, machine).await?.into_iter().filter(TvSection::is_music).collect())
    }

    /// The unreleased albums of each library (None: not read here).
    async fn unreleased(&self, machine: &str, sections: &[String]) -> AppResult<Vec<(String, Option<Vec<String>>)>> {
        let main = self.inner.host.main_section().await;
        let tag = crate::sid::tag(machine);
        let wanted: Vec<(String, String)> = sections
            .iter()
            .map(|s| (s.clone(), if main.as_deref() == Some(s.as_str()) { MAIN_SRC.to_string() } else { store::src_of(&tag, s) }))
            .collect();
        self.inner
            .db
            .read_async(move |c| {
                let mut out = Vec::new();
                for (section, src) in wanted {
                    if store::sync_state_in(c, &src)?.is_none() {
                        out.push((section, None));
                        continue;
                    }
                    let ids: Vec<String> = {
                        let mut st = c.prepare_cached("SELECT id FROM albums WHERE src = ?1 ORDER BY id")?;
                        st.query_map([&src], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?
                    };
                    let mut hits = Vec::new();
                    for id in ids {
                        if crate::platform::discord::commands::album_unreleased(c, &id)?.unwrap_or(true) {
                            hits.push(id);
                        }
                    }
                    out.push((section, Some(hits)));
                }
                Ok(out)
            })
            .await
    }

    /// My server's music libraries, how many unreleased albums each holds,
    /// and whether this account has Plex Pass.
    pub async fn overview(&self) -> AppResult<SharingOverview> {
        let server = self.inner.host.my_server().await;
        let Some(server) = server.clone().filter(|s| s.owned) else {
            return Ok(SharingOverview { server_name: server.map(|s| s.name), owner: false, plex_pass: None, libraries: Vec::new() });
        };
        let token = self.token().await?;
        let music = self.music(&token, &server.machine_id).await?;
        let keys: Vec<String> = music.iter().map(|s| s.key.clone()).collect();
        let counts = self.unreleased(&server.machine_id, &keys).await?;
        let defaults = self.inner.settings.default_libraries.read().clone();
        let plex_pass = self.inner.host.tv().plex_pass(&token).await.ok();
        let libraries = music
            .into_iter()
            .zip(counts)
            .map(|(s, (_, ids))| LibraryChoice { default: defaults.contains(&s.key), unreleased: ids.map(|v| v.len() as u32), id: s.key, title: s.title })
            .collect();
        Ok(SharingOverview { server_name: Some(server.name), owner: true, plex_pass, libraries })
    }

    /// Share my music with someone (a Plex username or email). Only music
    /// libraries are shared; unreleased albums are labelled and left out
    /// first when hiding is on.
    pub async fn share(&self, invitee: &str, opts: &ShareOptions) -> AppResult<ShareStatus> {
        let server = self.owned_server().await?;
        let token = self.token().await?;
        let mut chosen: Vec<String> = Vec::new();
        for l in &opts.libraries {
            if !chosen.contains(l) {
                chosen.push(l.clone());
            }
        }
        if chosen.is_empty() {
            return Err(AppError::Invalid("Pick a music library to share.".into()));
        }
        let music = match self.music(&token, &server.machine_id).await {
            Ok(m) => m,
            Err(e) if plex_down(&e) => return Ok(ShareStatus::PlexDown { message: "Plex isn't answering right now. Try again in a while.".into() }),
            Err(e) => return Err(e),
        };
        if let Some(bad) = chosen.iter().find(|l| !music.iter().any(|m| &m.key == *l)) {
            tracing::debug!(section = %bad, "not a music library on this server");
            return Err(AppError::Invalid("Only music libraries can be shared from Platter.".into()));
        }
        let mut exclude = Vec::new();
        let mut hidden = 0u32;
        if opts.hide_unreleased {
            let found = self.unreleased(&server.machine_id, &chosen).await?;
            let with: Vec<&(String, Option<Vec<String>>)> = found.iter().filter(|(_, ids)| ids.as_ref().is_none_or(|v| !v.is_empty())).collect();
            let pass = if with.is_empty() { None } else { Some(self.inner.host.tv().plex_pass(&token).await) };
            match pass {
                None => {}
                Some(Err(e)) if plex_down(&e) => return Ok(ShareStatus::PlexDown { message: "Plex isn't answering right now. Try again in a while.".into() }),
                Some(Err(e)) => return Err(e),
                Some(Ok(false)) => {
                    let title = |k: &str| music.iter().find(|m| m.key == k).map(|m| m.title.clone()).unwrap_or_default();
                    return Ok(ShareStatus::NeedsPlexPass {
                        with_unreleased: with.iter().map(|(k, _)| title(k)).collect(),
                        others: chosen.iter().filter(|k| !with.iter().any(|(w, _)| w == *k)).cloned().collect(),
                    });
                }
                Some(Ok(true)) => {
                    // A library Platter hasn't read can't be checked: refuse rather than guess.
                    if let Some((k, _)) = with.iter().find(|(_, ids)| ids.is_none()) {
                        let t = music.iter().find(|m| &m.key == k).map(|m| m.title.clone()).unwrap_or_default();
                        return Ok(ShareStatus::Refused { message: format!("Platter hasn't read {t} yet, so it can't keep its unreleased music out. Include it in Settings, Library, then share.") });
                    }
                    hidden = match self.label(&with).await {
                        Ok(n) => n,
                        Err(e) => {
                            tracing::warn!(kind = e.kind(), "unreleased albums not labelled; nothing shared");
                            return Ok(ShareStatus::Refused { message: "Platter couldn't mark your unreleased albums on the server, so nothing was shared.".into() });
                        }
                    };
                }
            }
            // With Plex Pass the filter stays on the share, so albums
            // labelled later are kept out too.
            if matches!(pass, Some(Ok(true))) || hidden > 0 {
                exclude.push(HIDE_LABEL.to_string());
            }
        }
        let terms = ShareTerms { invitee: invitee.trim().to_string(), sections: chosen, allow_downloads: opts.allow_downloads, exclude_labels: exclude };
        match self.inner.host.tv().share_server(&token, &server.machine_id, &terms).await {
            Ok(()) => {
                tracing::info!(hidden, "shared music with someone");
                Ok(ShareStatus::Invited { hidden })
            }
            Err(e) if is_already_shared(&e) => Ok(ShareStatus::AlreadyShared),
            Err(AppError::NotFound(m)) => Ok(ShareStatus::UnknownUser { message: m }),
            Err(e) if plex_down(&e) => Ok(ShareStatus::PlexDown { message: "Plex isn't answering right now. Try again in a while.".into() }),
            Err(AppError::Invalid(m)) if m.contains("username or email") => Ok(ShareStatus::UnknownUser { message: m }),
            Err(AppError::Invalid(m)) => Ok(ShareStatus::Refused { message: m }),
            Err(e) => Err(e),
        }
    }

    /// Label every unreleased album in these libraries (skipping the ones
    /// Platter labelled before). The number labelled or already labelled.
    async fn label(&self, libs: &[&(String, Option<Vec<String>>)]) -> AppResult<u32> {
        let done: HashSet<String> = self
            .inner
            .db
            .read_async(|c| Ok(store::kv_get(c, LABELLED_KEY)?.and_then(|v| serde_json::from_str::<Vec<String>>(&v).ok()).unwrap_or_default()))
            .await?
            .into_iter()
            .collect();
        let mut all = done.clone();
        let mut n = 0u32;
        let mut err = None;
        for (section, ids) in libs {
            for id in ids.iter().flatten() {
                n += 1;
                if done.contains(id) {
                    continue;
                }
                match self.inner.host.add_label(section, id, HIDE_LABEL).await {
                    Ok(_) => {
                        all.insert(id.clone());
                    }
                    Err(e) => {
                        err = Some(e);
                        break;
                    }
                }
            }
        }
        let mut list: Vec<String> = all.into_iter().collect();
        list.sort();
        let text = serde_json::to_string(&list).unwrap_or_default();
        self.inner.db.write_async(move |c| store::kv_set(c, LABELLED_KEY, &text)).await?;
        match err {
            Some(e) => Err(e),
            None => Ok(n),
        }
    }

    /// Everyone my server is shared with, what they see and when they were
    /// last on it.
    pub async fn people(&self) -> AppResult<Vec<PersonWithAccess>> {
        let server = self.owned_server().await?;
        let token = self.token().await?;
        let tv = self.inner.host.tv();
        let shares: Vec<SharedWith> = tv.shares(&token, &server.machine_id).await?;
        let sections = tv.server_sections(&token, &server.machine_id).await.unwrap_or_default();
        let seen = tv.last_seen(&token).await.unwrap_or_default();
        let mut out: Vec<PersonWithAccess> = shares
            .into_iter()
            .map(|s| PersonWithAccess {
                libraries: s.sections.iter().map(|k| sections.iter().find(|x| &x.key == k).map(|x| x.title.clone()).unwrap_or_else(|| "Library".into())).collect(),
                allow_downloads: s.allow_downloads,
                unreleased_hidden: s.filter_music.contains(HIDE_LABEL),
                pending: s.pending,
                last_seen_at: seen.iter().find(|l| l.user_id == s.user_id && l.machine_id == server.machine_id).and_then(|l| l.at),
                id: s.id,
                name: s.name,
            })
            .collect();
        out.sort_by(|a, b| b.last_seen_at.cmp(&a.last_seen_at).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
        Ok(out)
    }

    /// Take someone's access away.
    pub async fn remove(&self, share_id: &str) -> AppResult<()> {
        let server = self.owned_server().await?;
        let token = self.token().await?;
        self.inner.host.tv().unshare(&token, &server.machine_id, share_id).await?;
        tracing::info!("removed someone's access to the server");
        Ok(())
    }

    // ---- invites to me --------------------------------------------------------

    /// Invites this account received (asks plex.tv when `fresh`). New ones
    /// are announced once with a notification, while "Accept invites inside
    /// Platter" is on.
    pub async fn invites(&self, fresh: bool) -> AppResult<Vec<ReceivedInvite>> {
        if !fresh {
            return Ok(self.inner.pending.lock().clone());
        }
        let token = self.token().await?;
        let list: Vec<ReceivedInvite> = self.inner.host.tv().invites_received(&token).await?.into_iter().filter(|i| i.server).collect();
        let changed = *self.inner.pending.lock() != list;
        *self.inner.pending.lock() = list.clone();
        if changed {
            self.inner.events.emit(EVENT_INVITES, serde_json::to_value(&list).unwrap_or_default());
        }
        if self.inner.settings.accept_in_app.load(Ordering::Relaxed) {
            let new: Vec<ReceivedInvite> = {
                let mut seen = self.inner.announced.lock();
                list.iter().filter(|i| seen.insert(i.id.clone())).cloned().collect()
            };
            for i in new {
                self.inner.notifier.notify(&format!("{} shared their music with you", i.from), "Open Platter to start listening from their server.");
            }
        }
        Ok(list)
    }

    /// Accept an invite: plex.tv adds the server, Platter adds its music
    /// libraries and reads them, and the connection race finds it.
    pub async fn accept(&self, invite_id: &str) -> AppResult<Accepted> {
        let token = self.token().await?;
        let found = self.inner.pending.lock().iter().find(|i| i.id == invite_id).cloned();
        let invite = match found {
            Some(i) => i,
            None => self.invites(true).await?.into_iter().find(|i| i.id == invite_id).ok_or_else(|| AppError::NotFound("That invite is gone. It may have been taken back.".into()))?,
        };
        let host = &self.inner.host;
        let before: HashSet<String> = host.servers().await.into_iter().map(|s| s.machine_id).collect();
        host.tv().accept_invite(&token, &invite).await?;
        self.inner.pending.lock().retain(|i| i.id != invite.id);
        let list = self.inner.pending.lock().clone();
        self.inner.events.emit(EVENT_INVITES, serde_json::to_value(&list).unwrap_or_default());
        tracing::info!("accepted a server invite");
        let after = host.refresh_servers().await?;
        let theirs: Vec<ServerEntry> = after.into_iter().filter(|s| !s.owned && (!before.contains(&s.machine_id) || invite.servers.contains(&s.name))).collect();
        let mut entries = Vec::new();
        for s in &theirs {
            match host.music_sections(&s.machine_id).await {
                Ok(list) => entries.extend(list.into_iter().map(|sec| format!("{}/{}", s.machine_id, sec.id))),
                Err(e) => tracing::info!(kind = e.kind(), "the new server's libraries aren't listed yet"),
            }
        }
        if !entries.is_empty() {
            host.include(entries.clone()).await?;
            if let Some(f) = self.inner.on_accepted.lock().clone() {
                f(entries.clone());
            }
        }
        Ok(Accepted { servers: theirs.into_iter().map(|s| s.name).collect(), libraries: entries.len() as u32 })
    }

    fn notice(&self, message: String) {
        self.inner.events.emit(EVENT_NOTICE, serde_json::to_value(Notice { message }).unwrap_or_default());
    }

    /// The options a one-click share uses: the default libraries and switches.
    pub fn default_options(&self) -> ShareOptions {
        let s = &self.inner.settings;
        ShareOptions {
            libraries: s.default_libraries.read().clone(),
            hide_unreleased: s.hide_unreleased.load(Ordering::Relaxed),
            allow_downloads: s.allow_downloads.load(Ordering::Relaxed),
        }
    }

    /// Words for a share's outcome (listen along and link answers).
    pub fn status_text(st: &ShareStatus) -> String {
        match st {
            ShareStatus::Invited { .. } => "Invited".into(),
            ShareStatus::AlreadyShared => "They already have access to your server.".into(),
            ShareStatus::NeedsPlexPass { .. } => "Your unreleased music can only be kept out with Plex Pass. Share from Settings, Sharing to choose what to do.".into(),
            ShareStatus::UnknownUser { message } | ShareStatus::Refused { message } | ShareStatus::PlexDown { message } => message.clone(),
        }
    }
}

#[async_trait]
impl ServerOffer for Sharing {
    async fn offered(&self) -> bool {
        self.inner.settings.offer.load(Ordering::Relaxed) && self.inner.host.my_server().await.is_some_and(|s| s.owned)
    }

    async fn invite(&self, plex_user: &str) -> OfferOutcome {
        let opts = self.default_options();
        if opts.libraries.is_empty() {
            return OfferOutcome::Failed("Pick the libraries you share in Settings, Sharing first.".into());
        }
        match self.share(plex_user, &opts).await {
            Ok(ShareStatus::Invited { .. }) => OfferOutcome::Invited,
            Ok(ShareStatus::AlreadyShared) => OfferOutcome::AlreadyShared,
            Ok(st) => OfferOutcome::Failed(Self::status_text(&st)),
            Err(e) => OfferOutcome::Failed(match e {
                AppError::Invalid(m) | AppError::NotFound(m) | AppError::Auth(m) => m,
                _ => "Plex isn't answering right now. Try again in a while.".into(),
            }),
        }
    }

    async fn me(&self) -> Option<String> {
        if !self.inner.settings.accept_in_app.load(Ordering::Relaxed) {
            return None;
        }
        self.inner.host.username().await
    }

    async fn shared(&self, host: &str) {
        // plex.tv can take a moment to list a new invite.
        for wait in [0u64, 2, 5, 10] {
            tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            match self.invites(true).await {
                Ok(list) if !list.is_empty() => return,
                Ok(_) => {}
                Err(e) => return tracing::debug!(kind = e.kind(), "invites not read"),
            }
        }
        self.notice(format!("{host} shared their server. If no invite shows here, it's in your email from Plex."));
    }
}

// ---- the app's Plex ---------------------------------------------------------------

/// Plex through the app's state, settings and library cache.
pub struct PlexHost {
    pub plex: crate::plex::PlexState,
    pub db: DbHandle,
    pub settings: crate::settings::SettingsState,
}

#[async_trait]
impl Host for PlexHost {
    fn tv(&self) -> &PlexTv {
        self.plex.plex_tv()
    }
    async fn token(&self) -> Option<Token> {
        self.plex.account_token().await
    }
    async fn my_server(&self) -> Option<MyServer> {
        let s = self.plex.auth_status().await.server?;
        Some(MyServer { machine_id: s.machine_id, name: s.name, owned: s.owned })
    }
    async fn main_section(&self) -> Option<String> {
        let db = self.db.clone();
        db.read_async(store::sync_state).await.ok().flatten().map(|s| s.section_id)
    }
    async fn username(&self) -> Option<String> {
        let token = self.plex.account_token().await?;
        self.plex.plex_tv().user(&token).await.ok().map(|u| u.username).filter(|u| !u.trim().is_empty())
    }
    async fn add_label(&self, section: &str, album_id: &str, label: &str) -> AppResult<bool> {
        self.plex.client().await?.add_label(section, album_id, label).await
    }
    async fn servers(&self) -> Vec<ServerEntry> {
        self.plex.account_servers().await.into_iter().map(|s| ServerEntry { machine_id: s.machine_id, name: s.name, owned: s.owned }).collect()
    }
    async fn refresh_servers(&self) -> AppResult<Vec<ServerEntry>> {
        Ok(self.plex.refresh_servers().await?.into_iter().map(|s| ServerEntry { machine_id: s.machine_id, name: s.name, owned: s.owned }).collect())
    }
    async fn music_sections(&self, machine_id: &str) -> AppResult<Vec<Section>> {
        let c = self.plex.client_for_tag(&crate::sid::tag(machine_id)).await?;
        Ok(c.music_sections().await?.into_iter().map(|s| Section { id: s.id, title: s.title }).collect())
    }
    async fn include(&self, entries: Vec<String>) -> AppResult<()> {
        let mut libs = self.settings.get().await?.libraries;
        for e in entries {
            if !libs.contains(&e) {
                libs.push(e);
            }
        }
        let patch = crate::settings::SettingsPatch { libraries: Some(libs), ..Default::default() };
        self.settings.set(patch).await?;
        if let Err(e) = self.db.start_sync(false) {
            tracing::warn!("library sync not started: {e}");
        }
        Ok(())
    }
}
