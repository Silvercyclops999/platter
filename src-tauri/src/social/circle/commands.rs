//! Tauri commands, and the glue to the library cache, playlist editing,
//! Plex Home and the server's accounts.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use tauri::State;

use super::blend::{self, Made};
use super::library;
use super::wire::{Ident, Role, Taste};
use super::{
    Account, BlendCreated, BlendPage, Circle, CircleLibrary, CircleStore, CircleView, InviteLink, JoinResult, OwnerPlaylists, People, Rows, SharedInfo, SharedPage,
};
use crate::db::DbHandle;
use crate::db::merge::norm;
use crate::db::playlists::Place;
use crate::error::{AppError, AppResult};
use crate::model::Track;
use crate::playlists::Playlists;
use crate::plex::PlexState;
use crate::social::activity::ActivitySource;

const KV: &str = "circle.state";

/// Kept in the library cache's key-value table.
pub struct KvStore(pub DbHandle);

impl CircleStore for KvStore {
    fn load(&self) -> Option<String> {
        self.0.read(|c| crate::db::store::kv_get(c, KV)).ok().flatten()
    }
    fn save(&self, json: &str) {
        let json = json.to_string();
        if let Err(e) = self.0.write(move |c| crate::db::store::kv_set(c, KV, &json)) {
            tracing::warn!(kind = e.kind(), "shared playlists and Blends not saved");
        }
    }
}

/// Your own Plex playlists through the app's playlist editing (so edits
/// queue for Plex like any other).
pub struct AppPlaylists {
    pub db: DbHandle,
    pub playlists: Playlists,
}

#[async_trait]
impl OwnerPlaylists for AppPlaylists {
    async fn rows(&self, id: &str) -> AppResult<Option<Rows>> {
        let id = id.to_string();
        self.db
            .read_async(move |c| {
                let id = crate::db::playlists::resolve_id(c, &id)?;
                let Some(p) = crate::db::playlists::playlist(c, &id)? else { return Ok(None) };
                let items = crate::db::playlists::items(c, &id)?;
                let mut seen: HashMap<String, u32> = HashMap::new();
                let rows = items
                    .into_iter()
                    .map(|i| {
                        // A row not in Plex yet has no id: name it by its song.
                        let key = if i.item_id.is_empty() {
                            let n = seen.entry(i.track_id.clone()).or_insert(0);
                            *n += 1;
                            format!("local:{}:{n}", i.track_id)
                        } else {
                            i.item_id
                        };
                        (key, i.track_id)
                    })
                    .collect();
                Ok(Some(Rows { title: p.title, summary: p.summary, rows }))
            })
            .await
    }
    async fn add(&self, id: &str, track_ids: Vec<String>) -> AppResult<()> {
        self.playlists.add(id.to_string(), track_ids, Place::End).await.map(|_| ())
    }
    async fn remove(&self, id: &str, item_ids: Vec<String>) -> AppResult<()> {
        self.playlists.remove(id.to_string(), item_ids).await.map(|_| ())
    }
}

/// The library cache.
pub struct DbLibrary {
    pub db: DbHandle,
    /// What "liked" means now (a rating).
    pub liked_min: Arc<dyn Fn() -> f64 + Send + Sync>,
}

#[async_trait]
impl CircleLibrary for DbLibrary {
    async fn describe(&self, ids: Vec<String>) -> HashMap<String, Ident> {
        self.db.read_async(move |c| library::describe(c, &ids)).await.unwrap_or_default()
    }

    async fn resolve(&self, songs: Vec<Ident>) -> Vec<Option<String>> {
        let n = songs.len();
        self.db
            .read_async(move |c| Ok(songs.iter().map(|s| library::resolve(c, s).ok().flatten()).collect::<Vec<_>>()))
            .await
            .unwrap_or_else(|_| vec![None; n])
    }

    async fn taste(&self, now_s: i64, unreleased: bool) -> Taste {
        let min = (self.liked_min)();
        self.db
            .read_async(move |c| {
                let plays = library::plays(c, now_s - blend::WEEKS * 7 * 86_400)?;
                let mut ids: Vec<String> = plays.iter().map(|p| p.track_id.clone()).collect::<HashSet<_>>().into_iter().collect();
                let favs = library::favourites(c, min, blend::FAVOURITES * 2)?;
                ids.extend(favs.iter().cloned());
                let who = library::describe(c, &ids)?;
                let favs: Vec<Ident> = favs.iter().filter_map(|f| who.get(f).cloned()).collect();
                Ok(blend::taste(&plays, &who, &favs, now_s, unreleased))
            })
            .await
            .unwrap_or_default()
    }

    async fn blend(&self, members: Vec<(String, Taste)>, hidden: HashSet<String>, unreleased: bool, seed: u64, me: String) -> Made {
        self.db
            .read_async(move |c| {
                let refs: Vec<(String, &Taste)> = members.iter().map(|(i, t)| (i.clone(), t)).collect();
                let mut find = |s: &Ident| library::resolve(c, s).ok().flatten();
                let mut artist = |a: &str| {
                    let ids = library::artist_tracks(c, a, blend::PER_ARTIST * 2).unwrap_or_default();
                    if unreleased {
                        return ids;
                    }
                    // Unreleased songs stay out unless that's on.
                    let who = library::describe(c, &ids).unwrap_or_default();
                    ids.into_iter().filter(|i| who.get(i).is_some_and(|s| !s.u)).collect()
                };
                Ok(blend::generate(&refs, &mut find, &mut artist, &hidden, unreleased, seed, &me))
            })
            .await
            .unwrap_or_default()
    }

    async fn tracks(&self, ids: Vec<String>) -> Vec<Track> {
        self.db.read_async(move |c| crate::db::queries::tracks_by_ids(c, &ids)).await.unwrap_or_default()
    }
}

/// People on this server and Plex Home.
pub struct PlexPeople {
    pub plex: PlexState,
    pub home: crate::plex::home::Home,
    pub activity: Arc<dyn ActivitySource>,
}

fn user_key(name: &str) -> String {
    format!("user:{}", norm(name).replace(' ', ""))
}

#[async_trait]
impl People for PlexPeople {
    async fn me(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(h) = self.home.users().await
            && let Some(me) = h.users.iter().find(|u| u.current)
        {
            out.push(format!("home:{}", me.uuid));
        }
        if let Some(n) = self.plex.auth_status().await.username.filter(|n| !n.trim().is_empty()) {
            out.push(user_key(&n));
        }
        out
    }

    async fn others(&self) -> Vec<Account> {
        let mut out: Vec<Account> = Vec::new();
        if let Ok(h) = self.home.users().await {
            out.extend(h.users.iter().filter(|u| !u.current).map(|u| Account { key: format!("home:{}", u.uuid), name: u.name.clone(), avatar: u.avatar.clone(), home: true }));
        }
        // Friends on a server you own (Plex only lists them to its owner).
        if self.activity.owned().await == Some(true)
            && let Ok(body) = self.activity.accounts().await
            && let Ok(accounts) = crate::social::activity::parse_accounts(&body)
        {
            let me = self.plex.auth_status().await.username.map(|n| norm(&n)).unwrap_or_default();
            for a in accounts.into_iter().filter(|a| a.id != crate::social::activity::OWNER_ACCOUNT && norm(&a.name) != me) {
                if !out.iter().any(|o| norm(&o.name) == norm(&a.name)) {
                    out.push(Account { key: user_key(&a.name), name: a.name, avatar: a.thumb, home: false });
                }
            }
        }
        out
    }

    async fn server(&self) -> Option<String> {
        self.plex.machine_id().await
    }
}

// ---- commands -----------------------------------------------------------------

fn role(s: &str) -> AppResult<Role> {
    match s {
        "collaborator" => Ok(Role::Collaborator),
        "listener" => Ok(Role::Listener),
        _ => Err(AppError::Invalid("Choose collaborator or listener.".into())),
    }
}

#[tauri::command]
pub async fn circle_state(circle: State<'_, Circle>) -> AppResult<CircleView> {
    Ok(circle.view())
}

/// People on your server or Home you can invite without a link.
#[tauri::command]
pub async fn circle_people(circle: State<'_, Circle>) -> AppResult<Vec<Account>> {
    Ok(circle.people().await)
}

/// Join a playlist or Blend from its invite link.
#[tauri::command]
pub async fn circle_join(circle: State<'_, Circle>, link: String) -> AppResult<JoinResult> {
    circle.join(&link).await
}

/// Join an invite that came straight to you.
#[tauri::command]
pub async fn circle_accept(circle: State<'_, Circle>, id: String) -> AppResult<JoinResult> {
    circle.accept(&id).await
}

#[tauri::command]
pub async fn circle_dismiss(circle: State<'_, Circle>, id: String) -> AppResult<()> {
    circle.dismiss(&id).await;
    Ok(())
}

/// The owner's view of their shared playlist (None: not shared).
#[tauri::command]
pub async fn collab_info(circle: State<'_, Circle>, playlist_id: String) -> AppResult<Option<SharedInfo>> {
    circle.info(&playlist_id).await
}

#[tauri::command]
pub async fn collab_invite_link(circle: State<'_, Circle>, playlist_id: String, role: String) -> AppResult<InviteLink> {
    circle.invite_link(&playlist_id, self::role(&role)?).await
}

#[tauri::command]
pub async fn collab_invite_person(circle: State<'_, Circle>, playlist_id: String, key: String, role: String) -> AppResult<()> {
    circle.invite_person(&playlist_id, &key, self::role(&role)?).await
}

/// A playlist shared with you (`shared:<group>`).
#[tauri::command]
pub async fn collab_page(circle: State<'_, Circle>, id: String) -> AppResult<SharedPage> {
    circle.page(&id).await
}

#[tauri::command]
pub async fn collab_add(circle: State<'_, Circle>, id: String, track_ids: Vec<String>) -> AppResult<()> {
    circle.add(&id, track_ids).await
}

#[tauri::command]
pub async fn collab_remove(circle: State<'_, Circle>, id: String, keys: Vec<String>) -> AppResult<()> {
    circle.remove(&id, keys).await
}

#[tauri::command]
pub async fn collab_leave(circle: State<'_, Circle>, id: String) -> AppResult<()> {
    circle.leave(&id).await
}

#[tauri::command]
pub async fn collab_set_in_library(circle: State<'_, Circle>, id: String, on: bool) -> AppResult<()> {
    circle.set_in_library(&id, on).await
}

#[tauri::command]
pub async fn collab_set_role(circle: State<'_, Circle>, playlist_id: String, person_id: String, role: String) -> AppResult<()> {
    circle.set_role(&playlist_id, &person_id, self::role(&role)?).await
}

#[tauri::command]
pub async fn collab_remove_person(circle: State<'_, Circle>, playlist_id: String, person_id: String) -> AppResult<()> {
    circle.remove_person(&playlist_id, &person_id).await
}

#[tauri::command]
pub async fn collab_set_public(circle: State<'_, Circle>, playlist_id: String, public: bool) -> AppResult<()> {
    circle.set_public(&playlist_id, public).await
}

#[tauri::command]
pub async fn blend_create(circle: State<'_, Circle>, keys: Vec<String>) -> AppResult<BlendCreated> {
    circle.blend_create(keys).await
}

#[tauri::command]
pub async fn blend_invite_link(circle: State<'_, Circle>, id: String) -> AppResult<InviteLink> {
    circle.blend_invite(&id).await
}

#[tauri::command]
pub async fn blend_invite_person(circle: State<'_, Circle>, id: String, key: String) -> AppResult<()> {
    circle.blend_invite_person(&id, &key).await
}

#[tauri::command]
pub async fn blend_page(circle: State<'_, Circle>, id: String) -> AppResult<BlendPage> {
    circle.blend_page(&id).await
}

#[tauri::command]
pub async fn blend_hide(circle: State<'_, Circle>, id: String, track_id: String) -> AppResult<()> {
    circle.blend_hide(&id, &track_id).await
}

#[tauri::command]
pub async fn blend_refresh(circle: State<'_, Circle>, id: String) -> AppResult<BlendPage> {
    circle.blend_refresh(&id).await
}

#[tauri::command]
pub async fn blend_leave(circle: State<'_, Circle>, id: String) -> AppResult<()> {
    circle.blend_leave(&id).await
}
