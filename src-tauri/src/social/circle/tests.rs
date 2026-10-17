//! Collaborative playlists and Blends between in-process Platters through
//! a fake ntfy server on this computer (never the real ntfy.sh). Invented
//! people and songs only.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::sync::mpsc;

use super::blend::{self, Made};
use super::library::Play;
use super::wire::{Ident, Role, Taste};
use super::*;
use crate::db::merge::norm;
use crate::model::Track;
use crate::plex::state::EventSink;
use crate::remote::peer::auth::now_ms;
use crate::social::listen::fake_ntfy::FakeNtfy;
use crate::social::listen::relay::{Ntfy, Relay, RelayError, RelayEvent, RelayFactory, RelayHealth};

// ---- fakes ------------------------------------------------------------------------

#[derive(Default)]
struct Events(Mutex<Vec<(String, serde_json::Value)>>);

impl EventSink for Events {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.0.lock().push((event.to_string(), payload));
    }
}

impl Events {
    fn notices(&self) -> Vec<String> {
        self.0.lock().iter().filter(|(e, _)| e == EVENT_NOTICE).map(|(_, v)| v["message"].as_str().unwrap_or_default().to_string()).collect()
    }
}

/// A library: songs by id, and plays for the taste summary.
#[derive(Default)]
struct Lib {
    songs: Mutex<HashMap<String, Ident>>,
    plays: Mutex<Vec<Play>>,
}

impl Lib {
    fn add(&self, id: &str, title: &str, artist: &str, unreleased: bool) {
        let s = Ident { n: title.into(), a: artist.into(), al: Some("Album".into()), d: 200_000, k: Some(id.into()), u: unreleased, ..Default::default() };
        self.songs.lock().insert(id.into(), s);
    }
    fn play(&self, id: &str, times: usize) {
        let now = (now_ms() / 1000) as i64;
        for i in 0..times {
            self.plays.lock().push(Play { track_id: id.into(), at: now - 3600 - i as i64 });
        }
    }
    fn find(&self, s: &Ident) -> Option<String> {
        let songs = self.songs.lock();
        if let Some(k) = &s.k
            && songs.contains_key(k)
        {
            return Some(k.clone());
        }
        if s.u {
            return None;
        }
        let mut hits: Vec<&String> = songs.iter().filter(|(_, x)| !x.u && x.key() == s.key()).map(|(id, _)| id).collect();
        hits.sort();
        hits.first().map(|s| s.to_string())
    }
}

#[async_trait]
impl CircleLibrary for Lib {
    async fn describe(&self, ids: Vec<String>) -> HashMap<String, Ident> {
        let songs = self.songs.lock();
        ids.into_iter().filter_map(|i| songs.get(&i).map(|s| (i, s.clone()))).collect()
    }
    async fn resolve(&self, songs: Vec<Ident>) -> Vec<Option<String>> {
        songs.iter().map(|s| self.find(s)).collect()
    }
    async fn taste(&self, now_s: i64, unreleased: bool) -> Taste {
        let songs = self.songs.lock().clone();
        blend::taste(&self.plays.lock(), &songs, &[], now_s, unreleased)
    }
    async fn blend(&self, members: Vec<(String, Taste)>, hidden: HashSet<String>, unreleased: bool, seed: u64, me: String) -> Made {
        let refs: Vec<(String, &Taste)> = members.iter().map(|(i, t)| (i.clone(), t)).collect();
        let mut find = |s: &Ident| self.find(s);
        let songs = self.songs.lock().clone();
        let mut artist = |a: &str| {
            let mut v: Vec<String> = songs.iter().filter(|(_, s)| norm(&s.a) == norm(a) && (unreleased || !s.u)).map(|(i, _)| i.clone()).collect();
            v.sort();
            v
        };
        blend::generate(&refs, &mut find, &mut artist, &hidden, unreleased, seed, &me)
    }
    async fn tracks(&self, ids: Vec<String>) -> Vec<Track> {
        let songs = self.songs.lock();
        let al = crate::db::fake::album("al", "Album", &crate::db::fake::artist("ar", "Artist"), 2020, None, &[]);
        ids.iter()
            .filter_map(|i| {
                songs.get(i).map(|s| {
                    let mut t = crate::db::fake::track(i, &s.n, &al, 1, 1);
                    t.artist_title = s.a.clone();
                    t
                })
            })
            .collect()
    }
}

/// The owner's Plex playlists.
#[derive(Default)]
struct Pl {
    lists: Mutex<HashMap<String, Rows>>,
    next: AtomicU32,
}

impl Pl {
    fn make(&self, id: &str, title: &str, songs: &[&str]) {
        let rows = songs.iter().map(|t| (format!("i{}", self.next.fetch_add(1, Ordering::SeqCst)), t.to_string())).collect();
        self.lists.lock().insert(id.into(), Rows { title: title.into(), summary: None, rows });
    }
    fn songs(&self, id: &str) -> Vec<String> {
        self.lists.lock().get(id).map(|r| r.rows.iter().map(|(_, t)| t.clone()).collect()).unwrap_or_default()
    }
}

#[async_trait]
impl OwnerPlaylists for Pl {
    async fn rows(&self, id: &str) -> AppResult<Option<Rows>> {
        Ok(self.lists.lock().get(id).cloned())
    }
    async fn add(&self, id: &str, track_ids: Vec<String>) -> AppResult<()> {
        let mut l = self.lists.lock();
        let r = l.get_mut(id).ok_or_else(|| AppError::NotFound("playlist".into()))?;
        for t in track_ids {
            r.rows.push((format!("i{}", self.next.fetch_add(1, Ordering::SeqCst)), t));
        }
        Ok(())
    }
    async fn remove(&self, id: &str, item_ids: Vec<String>) -> AppResult<()> {
        let mut l = self.lists.lock();
        let r = l.get_mut(id).ok_or_else(|| AppError::NotFound("playlist".into()))?;
        r.rows.retain(|(i, _)| !item_ids.contains(i));
        Ok(())
    }
}

struct Ppl {
    me: Vec<String>,
    others: Vec<Account>,
    server: Option<String>,
}

#[async_trait]
impl People for Ppl {
    async fn me(&self) -> Vec<String> {
        self.me.clone()
    }
    async fn others(&self) -> Vec<Account> {
        self.others.clone()
    }
    async fn server(&self) -> Option<String> {
        self.server.clone()
    }
}

/// A relay this one computer can lose (its network down) while others keep theirs.
struct Gate {
    inner: Ntfy,
    off: Arc<AtomicBool>,
}

struct GateRelay {
    inner: Arc<dyn Relay>,
    off: Arc<AtomicBool>,
    published: Arc<AtomicU32>,
}

impl RelayFactory for Gate {
    fn relay(&self, base: &str) -> Arc<dyn Relay> {
        Arc::new(GateRelay { inner: self.inner.relay(base), off: self.off.clone(), published: Arc::default() })
    }
}

#[async_trait]
impl Relay for GateRelay {
    async fn publish(&self, topic: &str, body: &str) -> Result<(), RelayError> {
        if self.off.load(Ordering::SeqCst) {
            return Err(RelayError::Unreachable);
        }
        self.published.fetch_add(1, Ordering::SeqCst);
        self.inner.publish(topic, body).await
    }
    async fn subscribe(&self, topics: &[String], since: Option<&str>) -> Result<mpsc::Receiver<RelayEvent>, RelayError> {
        if self.off.load(Ordering::SeqCst) {
            return Err(RelayError::Unreachable);
        }
        let mut rx = self.inner.subscribe(topics, since).await?;
        let (tx, out) = mpsc::channel(64);
        let off = self.off.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    ev = rx.recv() => match ev {
                        Some(ev) => if tx.send(ev).await.is_err() { return },
                        None => return,
                    },
                    _ = tokio::time::sleep(Duration::from_millis(20)) => if off.load(Ordering::SeqCst) { return },
                }
            }
        });
        Ok(out)
    }
}

struct Node {
    c: Circle,
    lib: Arc<Lib>,
    pl: Arc<Pl>,
    events: Arc<Events>,
    off: Arc<AtomicBool>,
    /// Added to this computer's clock.
    ahead: Arc<AtomicU64>,
    tap: Arc<Mutex<Vec<String>>>,
    settings: CircleSettings,
}

fn cfg() -> CircleConfig {
    CircleConfig {
        tick: Duration::from_millis(30),
        resend_first: Duration::from_millis(120),
        resend_max: Duration::from_millis(400),
        answer_timeout: Duration::from_secs(4),
        retry: Duration::from_millis(40),
        retry_max: Duration::from_millis(200),
        taste_every: Duration::from_secs(3600),
    }
}

const SERVER: &str = "demo-machine-0001";

async fn node(ntfy: &FakeNtfy, name: &str, me: &[&str], others: Vec<Account>) -> Node {
    let lib = Arc::new(Lib::default());
    let pl = Arc::new(Pl::default());
    let events = Arc::new(Events::default());
    let off = Arc::new(AtomicBool::new(false));
    let ahead = Arc::new(AtomicU64::new(0));
    let tap: Arc<Mutex<Vec<String>>> = Arc::default();
    let settings = CircleSettings::default();
    let n = name.to_string();
    let a = ahead.clone();
    let t = tap.clone();
    let c = Circle::new(CircleDeps {
        relays: Arc::new(Gate { inner: Ntfy::new().unwrap(), off: off.clone() }),
        events: events.clone(),
        store: Arc::new(MemStore::default()),
        library: lib.clone(),
        playlists: pl.clone(),
        people: Arc::new(Ppl { me: me.iter().map(|s| s.to_string()).collect(), others, server: Some(SERVER.into()) }),
        name: Arc::new(move || n.clone()),
        avatar: Arc::new(|| Some("/ext/plex.tv/users/demo/avatar".into())),
        settings: settings.clone(),
        default_relay: ntfy.url.clone(),
        cfg: cfg(),
        clock: Arc::new(move || now_ms() + a.load(Ordering::SeqCst)),
        tap: Some(Arc::new(move |s: &str| t.lock().push(s.to_string()))),
    });
    c.start().await;
    Node { c, lib, pl, events, off, ahead, tap, settings }
}

async fn until<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < deadline {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    panic!("timed out waiting for: {what}");
}

fn joined_id(r: JoinResult) -> String {
    match r {
        JoinResult::Joined { id, .. } => id,
        other => panic!("not in: {other:?}"),
    }
}

/// Robin owns "Road Trip" (two songs on the shared server) and invites
/// Jules as a collaborator and Casey as a listener.
async fn shared_trio(ntfy: &FakeNtfy) -> (Node, Node, Node, String, String) {
    let robin = node(ntfy, "Robin", &[], vec![]).await;
    for (id, t) in [("aaaaaaaa:1", "First Light"), ("aaaaaaaa:2", "Second Wind"), ("aaaaaaaa:3", "Third Coast")] {
        robin.lib.add(id, t, "The Demo Band", false);
    }
    robin.pl.make("aaaaaaaa:p1", "Road Trip", &["aaaaaaaa:1", "aaaaaaaa:2"]);
    let jules = node(ntfy, "Jules", &[], vec![]).await;
    let casey = node(ntfy, "Casey", &[], vec![]).await;
    for n in [&jules, &casey] {
        // Same server: the same ids. Third Coast is there too.
        for (id, t) in [("aaaaaaaa:1", "First Light"), ("aaaaaaaa:2", "Second Wind"), ("aaaaaaaa:3", "Third Coast")] {
            n.lib.add(id, t, "The Demo Band", false);
        }
    }
    let l1 = robin.c.invite_link("aaaaaaaa:p1", Role::Collaborator).await.unwrap();
    let j = joined_id(jules.c.join(&l1.link).await.unwrap());
    let l2 = robin.c.invite_link("aaaaaaaa:p1", Role::Listener).await.unwrap();
    let c = joined_id(casey.c.join(&l2.link).await.unwrap());
    assert_eq!(j, c, "the same playlist");
    (robin, jules, casey, j, "aaaaaaaa:p1".to_string())
}

async fn titles(n: &Node, id: &str) -> Vec<String> {
    n.c.page(id).await.map(|p| p.rows.iter().filter(|r| !r.pending).map(|r| r.title.clone()).collect()).unwrap_or_default()
}

// ---- collaborative playlists ------------------------------------------------------------

#[tokio::test]
async fn members_join_with_their_roles_and_see_the_playlist() {
    let ntfy = FakeNtfy::start().await;
    let (robin, jules, casey, id, pl) = shared_trio(&ntfy).await;
    assert!(id.starts_with(SHARED_PREFIX));
    until("jules sees the songs", || async { titles(&jules, &id).await == ["First Light", "Second Wind"] }).await;
    until("casey too", || async { titles(&casey, &id).await.len() == 2 }).await;
    let s = jules.c.shared(&id).await.unwrap();
    assert_eq!((s.role, s.owner.name.as_str()), (Role::Collaborator, "Robin"));
    until("everyone is listed", || async { casey.c.shared(&id).await.unwrap().people.len() == 3 }).await;
    let s = casey.c.shared(&id).await.unwrap();
    assert_eq!(s.role, Role::Listener);
    let me = s.people.iter().find(|p| p.you).unwrap();
    assert_eq!((me.name.as_str(), me.role), ("Casey", Some(Role::Listener)));
    // The owner sees who's in.
    let o = robin.c.shared(&pl).await.unwrap();
    assert_eq!(o.people.iter().map(|p| (p.name.as_str(), p.role)).collect::<Vec<_>>(), vec![("Robin", Some(Role::Owner)), ("Jules", Some(Role::Collaborator)), ("Casey", Some(Role::Listener))]);
}

#[tokio::test]
async fn a_collaborator_s_edit_is_applied_by_the_owner_and_reaches_everyone() {
    let ntfy = FakeNtfy::start().await;
    let (robin, jules, casey, id, pl) = shared_trio(&ntfy).await;
    until("jules has it", || async { titles(&jules, &id).await.len() == 2 }).await;
    jules.c.add(&id, vec!["aaaaaaaa:3".into()]).await.unwrap();
    // Shown at once as waiting for Robin.
    let p = jules.c.page(&id).await.unwrap();
    assert!(p.rows.iter().any(|r| r.pending && r.title == "Third Coast"));
    until("robin's playlist has it", || async { robin.pl.songs(&pl).contains(&"aaaaaaaa:3".to_string()) }).await;
    until("casey sees it live", || async { titles(&casey, &id).await.contains(&"Third Coast".to_string()) }).await;
    until("confirmed for jules", || async { jules.c.shared(&id).await.unwrap().waiting == 0 }).await;
    // Added by Jules, everywhere.
    let row = casey.c.page(&id).await.unwrap().rows.into_iter().find(|r| r.title == "Third Coast").unwrap();
    assert_eq!(row.by.map(|b| b.name), Some("Jules".to_string()));
    let info = robin.c.info(&pl).await.unwrap().unwrap();
    assert_eq!(info.added_by.values().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Jules"]);
    until("jules is counted", || async { casey.c.shared(&id).await.unwrap().people.iter().any(|p| p.name == "Jules" && p.added == 1) }).await;
    // Jules removes the song they added; not Robin's.
    let rows = jules.c.page(&id).await.unwrap().rows;
    let theirs = rows.iter().find(|r| r.title == "Third Coast").unwrap();
    assert!(theirs.removable && !rows.iter().find(|r| r.title == "First Light").unwrap().removable);
    assert!(jules.c.remove(&id, vec![rows[0].key.clone()]).await.is_err());
    jules.c.remove(&id, vec![theirs.key.clone()]).await.unwrap();
    until("gone at robin's", || async { !robin.pl.songs(&pl).contains(&"aaaaaaaa:3".to_string()) }).await;
    until("gone at casey's", || async { titles(&casey, &id).await.len() == 2 }).await;
    // A listener can't add.
    assert!(casey.c.add(&id, vec!["aaaaaaaa:3".into()]).await.is_err());
}

#[tokio::test]
async fn the_owner_s_own_changes_and_a_song_not_on_their_server() {
    let ntfy = FakeNtfy::start().await;
    let (robin, jules, _casey, id, pl) = shared_trio(&ntfy).await;
    until("jules has it", || async { titles(&jules, &id).await.len() == 2 }).await;
    // Robin edits the playlist in Platter: members follow.
    robin.pl.add(&pl, vec!["aaaaaaaa:3".into()]).await.unwrap();
    robin.c.kick();
    until("jules follows", || async { titles(&jules, &id).await == ["First Light", "Second Wind", "Third Coast"] }).await;
    // Jules adds a song from another server Robin doesn't have.
    jules.lib.add("bbbbbbbb:9", "Elsewhere", "Other Demo Artist", false);
    jules.c.add(&id, vec!["bbbbbbbb:9".into()]).await.unwrap();
    until("told why not", || async { jules.events.notices().iter().any(|n| n.contains("Robin doesn't have Elsewhere")) }).await;
    assert!(!robin.pl.songs(&pl).contains(&"bbbbbbbb:9".to_string()));
    until("no longer waiting", || async { jules.c.shared(&id).await.unwrap().waiting == 0 }).await;
}

#[tokio::test]
async fn a_song_someone_can_t_play_shows_as_not_in_their_library() {
    let ntfy = FakeNtfy::start().await;
    let robin = node(&ntfy, "Robin", &[], vec![]).await;
    robin.lib.add("aaaaaaaa:1", "First Light", "The Demo Band", false);
    robin.lib.add("aaaaaaaa:2", "Rare Song", "The Demo Band", false);
    robin.pl.make("aaaaaaaa:p1", "Road Trip", &["aaaaaaaa:1", "aaaaaaaa:2"]);
    // Morgan is on another server with their own copy of one song.
    let morgan = node(&ntfy, "Morgan", &[], vec![]).await;
    morgan.lib.add("cccccccc:5", "First Light", "The Demo Band", false);
    let l = robin.c.invite_link("aaaaaaaa:p1", Role::Listener).await.unwrap();
    let id = joined_id(morgan.c.join(&l.link).await.unwrap());
    until("rows arrive", || async { morgan.c.page(&id).await.map(|p| p.rows.len() == 2).unwrap_or(false) }).await;
    let rows = morgan.c.page(&id).await.unwrap().rows;
    assert_eq!(rows[0].track.as_ref().map(|t| t.id.as_str()), Some("cccccccc:5"), "their own copy plays");
    assert!(rows[1].track.is_none(), "Not in your library");
    assert_eq!(rows[1].title, "Rare Song");
}

#[tokio::test]
async fn edits_wait_while_the_owner_is_offline_and_go_when_they_re_back() {
    let ntfy = FakeNtfy::start().await;
    let (robin, jules, casey, id, pl) = shared_trio(&ntfy).await;
    until("jules has it", || async { titles(&jules, &id).await.len() == 2 }).await;
    robin.off.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(100)).await;
    jules.c.add(&id, vec!["aaaaaaaa:3".into()]).await.unwrap();
    // Sent again and again while nobody answers.
    let edits = || jules.tap.lock().iter().filter(|p| p.contains("\"k\":\"edit\"")).count();
    until("sent more than once", || async { edits() >= 3 }).await;
    until("robin shows as unreachable", || async { jules.c.shared(&id).await.unwrap().owner_unreachable }).await;
    assert!(!robin.pl.songs(&pl).contains(&"aaaaaaaa:3".to_string()));
    assert_eq!(jules.c.shared(&id).await.unwrap().waiting, 1);
    robin.off.store(false, Ordering::SeqCst);
    until("applied once robin is back", || async { robin.pl.songs(&pl).contains(&"aaaaaaaa:3".to_string()) }).await;
    until("acknowledged", || async { jules.c.shared(&id).await.unwrap().waiting == 0 }).await;
    until("casey sees it", || async { titles(&casey, &id).await.len() == 3 }).await;
    // Applied once, however often it was sent.
    assert_eq!(robin.pl.songs(&pl).iter().filter(|s| *s == "aaaaaaaa:3").count(), 1);
    assert!(!jules.c.shared(&id).await.unwrap().owner_unreachable);
}

#[tokio::test]
async fn edits_from_two_collaborators_land_in_the_owner_s_order() {
    let ntfy = FakeNtfy::start().await;
    let (robin, jules, casey, id, pl) = shared_trio(&ntfy).await;
    robin.c.set_role(&pl, &robin.c.shared(&pl).await.unwrap().people[2].id, Role::Collaborator).await.unwrap();
    until("casey can add", || async { casey.c.shared(&id).await.map(|s| s.role == Role::Collaborator).unwrap_or(false) }).await;
    for (n, i) in [(&jules, 4), (&casey, 5), (&jules, 6), (&casey, 7)] {
        let sid = format!("aaaaaaaa:{i}");
        for x in [&robin, &jules, &casey] {
            x.lib.add(&sid, &format!("Song {i}"), "The Demo Band", false);
        }
        n.c.add(&id, vec![sid]).await.unwrap();
    }
    until("all four in", || async { robin.pl.songs(&pl).len() == 6 }).await;
    let owner_order: Vec<String> = robin.pl.songs(&pl);
    // Each person's own adds stay in their order.
    let pos = |s: &str| owner_order.iter().position(|x| x == s).unwrap();
    assert!(pos("aaaaaaaa:4") < pos("aaaaaaaa:6") && pos("aaaaaaaa:5") < pos("aaaaaaaa:7"));
    // Everyone ends up with exactly the owner's playlist.
    let want: Vec<String> = owner_order.iter().map(|i| robin.lib.songs.lock()[i].n.clone()).collect();
    until("jules matches", || async { titles(&jules, &id).await == want }).await;
    until("casey matches", || async { titles(&casey, &id).await == want }).await;
}

#[tokio::test]
async fn invites_are_good_once_and_expire() {
    let ntfy = FakeNtfy::start().await;
    let (robin, _jules, casey, _id, pl) = shared_trio(&ntfy).await;
    let morgan = node(&ntfy, "Morgan", &[], vec![]).await;
    // Used.
    let l = robin.c.invite_link(&pl, Role::Listener).await.unwrap();
    joined_id(casey.c.join(&l.link).await.unwrap());
    let e = morgan.c.join(&l.link).await.unwrap_err();
    assert!(e.to_string().contains("already used"), "{e}");
    // Expired, seen by the joiner.
    let l = robin.c.invite_link(&pl, Role::Listener).await.unwrap();
    morgan.ahead.store(25 * 3_600_000, Ordering::SeqCst);
    let e = morgan.c.join(&l.link).await.unwrap_err();
    assert!(e.to_string().contains("expired"), "{e}");
    // Expired, seen by the owner (the joiner's clock is behind).
    morgan.ahead.store(0, Ordering::SeqCst);
    robin.ahead.store(25 * 3_600_000, Ordering::SeqCst);
    let e = morgan.c.join(&l.link).await.unwrap_err();
    assert!(e.to_string().contains("expired"), "{e}");
    // Not an invite.
    assert!(morgan.c.join("platter://join/AAAA").await.is_err());
}

#[tokio::test]
async fn joining_while_the_owner_is_offline_waits_and_finishes_later() {
    let ntfy = FakeNtfy::start().await;
    let robin = node(&ntfy, "Robin", &[], vec![]).await;
    robin.lib.add("aaaaaaaa:1", "First Light", "The Demo Band", false);
    robin.pl.make("aaaaaaaa:p1", "Road Trip", &["aaaaaaaa:1"]);
    let l = robin.c.invite_link("aaaaaaaa:p1", Role::Listener).await.unwrap();
    robin.off.store(true, Ordering::SeqCst);
    let jules = node(&ntfy, "Jules", &[], vec![]).await;
    jules.lib.add("aaaaaaaa:1", "First Light", "The Demo Band", false);
    let r = tokio::time::timeout(Duration::from_secs(10), jules.c.join(&l.link)).await.unwrap().unwrap();
    assert!(matches!(r, JoinResult::Waiting { .. }));
    assert_eq!(jules.c.view().joining, 1);
    robin.off.store(false, Ordering::SeqCst);
    until("in once robin is back", || async { jules.c.view().playlists.len() == 1 }).await;
    until("songs arrive", || async { jules.c.view().playlists[0].track_count == 1 }).await;
    assert_eq!(jules.c.view().joining, 0);
}

#[tokio::test]
async fn joining_with_the_relay_down_says_so() {
    let ntfy = FakeNtfy::start().await;
    let (robin, _j, _c, _id, pl) = shared_trio(&ntfy).await;
    let l = robin.c.invite_link(&pl, Role::Listener).await.unwrap();
    let morgan = node(&ntfy, "Morgan", &[], vec![]).await;
    morgan.off.store(true, Ordering::SeqCst);
    let e = morgan.c.join(&l.link).await.unwrap_err();
    assert!(e.to_string().contains("relay"), "{e}");
    until("shown as unreachable", || async { morgan.c.view().relay == RelayHealth::Unreachable }).await;
    assert_eq!(morgan.c.view().joining, 0);
}

#[tokio::test]
async fn roles_change_people_are_removed_and_leaving_depends_on_privacy() {
    let ntfy = FakeNtfy::start().await;
    let (robin, jules, casey, id, pl) = shared_trio(&ntfy).await;
    until("listed", || async { robin.c.shared(&pl).await.unwrap().people.len() == 3 }).await;
    let people = robin.c.shared(&pl).await.unwrap().people;
    let (jid, cid) = (people[1].id.clone(), people[2].id.clone());
    robin.c.set_role(&pl, &jid, Role::Listener).await.unwrap();
    until("jules is a listener", || async { jules.c.shared(&id).await.unwrap().role == Role::Listener }).await;
    // Taken off.
    robin.c.remove_person(&pl, &cid).await.unwrap();
    until("casey loses it", || async { casey.c.shared(&id).await.unwrap().gone }).await;
    assert!(casey.events.notices().iter().any(|n| n.contains("taken off")));
    until("jules sees one fewer", || async { jules.c.shared(&id).await.unwrap().people.len() == 2 }).await;
    // Public: leaving keeps it here, off the list.
    robin.c.set_public(&pl, true).await.unwrap();
    until("public", || async { jules.c.shared(&id).await.unwrap().public }).await;
    jules.c.leave(&id).await.unwrap();
    assert_eq!(jules.c.shared(&id).await.unwrap().role, Role::Follower);
    until("off robin's list", || async { robin.c.shared(&pl).await.unwrap().people.len() == 1 }).await;
    // Made private: followers lose it.
    robin.c.set_public(&pl, false).await.unwrap();
    until("jules loses it", || async { jules.c.shared(&id).await.unwrap().gone }).await;
}

#[tokio::test]
async fn leaving_a_private_playlist_and_the_owner_deleting_one() {
    let ntfy = FakeNtfy::start().await;
    let (robin, jules, casey, id, pl) = shared_trio(&ntfy).await;
    until("listed", || async { robin.c.shared(&pl).await.unwrap().people.len() == 3 }).await;
    jules.c.leave(&id).await.unwrap();
    assert!(jules.c.shared(&id).await.is_none(), "gone from jules at once");
    until("robin drops jules", || async { robin.c.shared(&pl).await.unwrap().people.len() == 2 }).await;
    // Removing it from the library keeps membership.
    casey.c.set_in_library(&id, false).await.unwrap();
    let s = casey.c.shared(&id).await.unwrap();
    assert!(!s.in_library && s.role == Role::Listener);
    // Robin deletes it: Casey sees it as unavailable and can remove it.
    robin.pl.lists.lock().remove(&pl);
    robin.c.kick();
    until("unavailable", || async { casey.c.shared(&id).await.unwrap().gone }).await;
    assert!(casey.events.notices().iter().any(|n| n.contains("deleted Road Trip")));
    casey.c.leave(&id).await.unwrap();
    assert!(casey.c.shared(&id).await.is_none());
}

#[tokio::test]
async fn only_metadata_goes_out_and_it_is_sealed() {
    let ntfy = FakeNtfy::start().await;
    let (robin, jules, casey, id, _pl) = shared_trio(&ntfy).await;
    robin.lib.add("aaaaaaaa:8", "Vault Take", "The Demo Band", true);
    robin.pl.add("aaaaaaaa:p1", vec!["aaaaaaaa:8".into()]).await.unwrap();
    until("jules has it", || async { titles(&jules, &id).await.len() == 2 }).await;
    jules.c.add(&id, vec!["aaaaaaaa:3".into()]).await.unwrap();
    until("casey sees it", || async { titles(&casey, &id).await.len() == 3 }).await;
    let all: Vec<String> = [&robin, &jules, &casey].iter().flat_map(|n| n.tap.lock().clone()).collect();
    assert!(!all.is_empty());
    for p in &all {
        for bad in ["/library", "file.flac", "token", "X-Plex", SERVER, "http", "127.0.0.1", "part"] {
            assert!(!p.contains(bad), "{bad} in {p}");
        }
        // Unreleased songs stay home while that's off.
        assert!(!p.contains("Vault Take"), "{p}");
    }
    // Through the relay: only sealed text, with listen along's ntfy headers.
    let published = ntfy.published();
    assert!(!published.is_empty());
    for m in &published {
        assert!(m.body.starts_with("1.") && !m.body.contains("Road Trip") && !m.body.contains("Robin"), "{}", m.body);
        assert!(m.headers.iter().any(|(k, v)| k == "x-firebase" && v == "no"));
        assert!(m.body.len() < crate::social::listen::seal::MAX_BODY);
    }
    // With it on, unreleased songs are shared.
    robin.settings.share_unreleased.store(true, Ordering::SeqCst);
    robin.c.kick();
    until("now shared", || async { titles(&casey, &id).await.contains(&"Vault Take".to_string()) }).await;
}

#[tokio::test]
async fn people_on_the_server_are_invited_without_a_link() {
    let ntfy = FakeNtfy::start().await;
    let jules_acct = Account { key: "home:jules-uuid".into(), name: "Jules".into(), avatar: None, home: true };
    let robin = node(&ntfy, "Robin", &["home:robin-uuid"], vec![jules_acct.clone()]).await;
    let robin_acct = Account { key: "home:robin-uuid".into(), name: "Robin".into(), avatar: None, home: true };
    let jules = node(&ntfy, "Jules", &["home:jules-uuid"], vec![robin_acct]).await;
    robin.lib.add("aaaaaaaa:1", "First Light", "The Demo Band", false);
    jules.lib.add("aaaaaaaa:1", "First Light", "The Demo Band", false);
    robin.pl.make("aaaaaaaa:p1", "Road Trip", &["aaaaaaaa:1"]);
    // Home users are listed to pick directly.
    assert_eq!(robin.c.people().await, vec![jules_acct]);
    tokio::time::sleep(Duration::from_millis(200)).await;
    robin.c.invite_person("aaaaaaaa:p1", "home:jules-uuid", Role::Collaborator).await.unwrap();
    until("it rings", || async { jules.c.view().incoming.len() == 1 }).await;
    let inc = jules.c.view().incoming[0].clone();
    assert_eq!((inc.from.as_str(), inc.title.as_str()), ("Robin", "Road Trip"));
    let id = joined_id(jules.c.accept(&inc.id).await.unwrap());
    assert!(jules.c.view().incoming.is_empty());
    until("a collaborator", || async { jules.c.shared(&id).await.map(|s| s.role == Role::Collaborator && s.track_count == 1).unwrap_or(false) }).await;
    // Off: no doorbell, no invites.
    jules.settings.playlists.store(false, Ordering::SeqCst);
    assert!(jules.c.join("platter://collab/AAAA").await.is_err());
}

#[tokio::test]
async fn a_ring_is_unconfirmed_and_a_borrowed_name_is_called_out_on_joining() {
    let ntfy = FakeNtfy::start().await;
    let robin_acct = Account { key: "home:robin-uuid".into(), name: "Robin".into(), avatar: None, home: true };
    let jules = node(&ntfy, "Jules", &["home:jules-uuid"], vec![robin_acct]).await;
    let mallory = node(&ntfy, "Mallory", &[], vec![]).await;
    mallory.lib.add("aaaaaaaa:1", "First Light", "The Demo Band", false);
    jules.lib.add("aaaaaaaa:1", "First Light", "The Demo Band", false);
    mallory.pl.make("aaaaaaaa:p9", "Free Songs", &["aaaaaaaa:1"]);
    let link = mallory.c.invite_link("aaaaaaaa:p9", Role::Listener).await.unwrap().link;
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Anyone who knows the server's id and Jules's key can ring, under any name.
    let ch = doorbell(SERVER, "home:jules-uuid");
    let p = Packet { f: String::new(), c: 0, fl: 0, m: Msg::Ring { link, from: "Robin".into(), kind: Kind::Playlist, title: "Road Trip".into() } };
    let relay = Ntfy::new().unwrap().relay(&ntfy.url);
    for b in link::seal_packet(&ch, &p).unwrap() {
        relay.publish(&ch.topic, &b).await.unwrap();
    }
    until("it rings", || async { jules.c.view().incoming.len() == 1 }).await;
    assert!(jules.events.notices().iter().any(|n| n == "An invite that says it's from Robin: the playlist Road Trip."), "{:?}", jules.events.notices());
    let inc = jules.c.view().incoming[0].clone();
    joined_id(jules.c.accept(&inc.id).await.unwrap());
    assert!(
        jules.events.notices().iter().any(|n| n.contains("said it was from Robin, but it's from Mallory")),
        "{:?}",
        jules.events.notices()
    );
}

// ---- Blends -------------------------------------------------------------------------------

/// A computer with plays of `n` songs by `artist`, `shared` of them on the demo server.
fn music(n: &Node, prefix: &str, artist: &str, count: usize) {
    for i in 0..count {
        let id = format!("{prefix}:{i}");
        n.lib.add(&id, &format!("{artist} {i}"), artist, false);
        n.lib.play(&id, count - i);
    }
}

#[tokio::test]
async fn a_blend_takes_turns_between_people_from_songs_each_can_play() {
    let ntfy = FakeNtfy::start().await;
    let robin = node(&ntfy, "Robin", &[], vec![]).await;
    let jules = node(&ntfy, "Jules", &[], vec![]).await;
    music(&robin, "aaaaaaaa", "Demo Band", 30);
    music(&jules, "bbbbbbbb", "Other Demo Artist", 30);
    // Each has some of the other's songs (partial overlap), on their own servers.
    for i in 0..10 {
        robin.lib.add(&format!("cccccccc:{i}"), &format!("Other Demo Artist {i}"), "Other Demo Artist", false);
        jules.lib.add(&format!("dddddddd:{i}"), &format!("Demo Band {i}"), "Demo Band", false);
    }
    let made = robin.c.blend_create(vec![]).await.unwrap();
    let id = joined_id(jules.c.join(&made.invite.link).await.unwrap());
    assert_eq!(id, made.id);
    until("both made with both tastes", || async {
        let ok = |p: AppResult<BlendPage>| p.map(|p| p.rows.iter().any(|r| r.by.name == "Robin") && p.rows.iter().any(|r| r.by.name == "Jules")).unwrap_or(false);
        ok(robin.c.blend_page(&id).await) && ok(jules.c.blend_page(&id).await)
    })
    .await;
    let r = robin.c.blend_page(&id).await.unwrap();
    assert_eq!(r.summary.title, "Robin + Jules");
    assert_eq!(r.summary.description, "A Blend of music for Robin and Jules");
    // Robin only gets songs Robin can play; Jules's others are missing.
    assert!(r.rows.iter().all(|x| robin.lib.songs.lock().contains_key(&x.track.id)));
    let theirs = r.rows.iter().filter(|x| x.by.name == "Jules").count();
    assert_eq!(theirs, 10, "the overlap");
    assert!(r.missing.iter().all(|m| m.by.name == "Jules") && r.missing.len() >= 10);
    assert!(r.rows.len() >= 40);
    // Only summaries went out.
    for p in robin.tap.lock().iter().chain(jules.tap.lock().iter()) {
        assert!(!p.contains("/library") && !p.contains(SERVER) && !p.contains("token"), "{p}");
    }
}

#[tokio::test]
async fn hidden_songs_stay_hidden_and_leaving_ends_a_blend_of_two() {
    let ntfy = FakeNtfy::start().await;
    let robin = node(&ntfy, "Robin", &[], vec![]).await;
    let jules = node(&ntfy, "Jules", &[], vec![]).await;
    music(&robin, "aaaaaaaa", "Demo Band", 20);
    music(&jules, "aaaaaaaa", "Demo Band", 20);
    let made = robin.c.blend_create(vec![]).await.unwrap();
    let id = joined_id(jules.c.join(&made.invite.link).await.unwrap());
    until("made", || async { jules.c.blend_page(&id).await.map(|p| !p.rows.is_empty()).unwrap_or(false) }).await;
    let first = jules.c.blend_page(&id).await.unwrap().rows[0].track.id.clone();
    jules.c.blend_hide(&id, &first).await.unwrap();
    assert!(!jules.c.blend_page(&id).await.unwrap().rows.iter().any(|r| r.track.id == first));
    let again = jules.c.blend_refresh(&id).await.unwrap();
    assert!(!again.rows.iter().any(|r| r.track.id == first), "doesn't come back");
    // A new day too.
    jules.ahead.store(86_400_000, Ordering::SeqCst);
    jules.c.tick().await;
    assert!(!jules.c.blend_page(&id).await.unwrap().rows.iter().any(|r| r.track.id == first));
    // Jules leaves: gone for Jules; Robin's Blend had only Jules, so it's removed.
    until("robin knows jules", || async { robin.c.view().blends.first().map(|b| b.people.len() == 2).unwrap_or(false) }).await;
    jules.c.blend_leave(&id).await.unwrap();
    assert!(jules.c.blend_page(&id).await.is_err());
    until("removed for robin", || async { robin.c.view().blends.is_empty() }).await;
}

#[tokio::test]
async fn unreleased_songs_stay_out_of_blends_and_tastes() {
    let ntfy = FakeNtfy::start().await;
    let robin = node(&ntfy, "Robin", &[], vec![]).await;
    let jules = node(&ntfy, "Jules", &[], vec![]).await;
    robin.lib.add("aaaaaaaa:1", "Vault Take", "Demo Band", true);
    robin.lib.play("aaaaaaaa:1", 9);
    music(&robin, "aaaaaaaa", "Demo Band", 5);
    jules.lib.add("aaaaaaaa:1", "Vault Take", "Demo Band", true);
    music(&jules, "aaaaaaaa", "Demo Band", 5);
    let made = robin.c.blend_create(vec![]).await.unwrap();
    let id = joined_id(jules.c.join(&made.invite.link).await.unwrap());
    until("made", || async { jules.c.blend_page(&id).await.map(|p| p.rows.iter().any(|r| r.by.name == "Robin")).unwrap_or(false) }).await;
    assert!(!jules.c.blend_page(&id).await.unwrap().rows.iter().any(|r| r.track.title == "Vault Take"));
    assert!(!robin.c.blend_page(&id).await.unwrap().rows.iter().any(|r| r.track.title == "Vault Take"));
    assert!(robin.tap.lock().iter().all(|p| !p.contains("Vault Take")));
}

#[tokio::test]
async fn a_blend_takes_ten_people_at_most() {
    let ntfy = FakeNtfy::start().await;
    let robin = node(&ntfy, "Robin", &[], vec![]).await;
    let keys: Vec<String> = (0..10).map(|i| format!("home:{i}")).collect();
    let e = robin.c.blend_create(keys).await.unwrap_err();
    assert!(e.to_string().contains("10 people"), "{e}");
    let made = robin.c.blend_create(vec![]).await.unwrap();
    let mut link = made.invite.link.clone();
    let mut nodes = Vec::new();
    for i in 0..9 {
        let n = node(&ntfy, &format!("Friend {i}"), &[], vec![]).await;
        joined_id(n.c.join(&link).await.unwrap());
        nodes.push(n);
        if i < 8 {
            link = robin.c.blend_invite(&made.id).await.unwrap().link;
        }
    }
    until("ten in", || async { robin.c.view().blends[0].people.len() == 10 }).await;
    assert!(robin.c.blend_invite(&made.id).await.unwrap_err().to_string().contains("full"));
}
