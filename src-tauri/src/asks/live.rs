//! The app's side of requests: Plex for who's signed in, sharing for who
//! the server is shared with, Lidarr for getting it, and the OS for
//! notifications.

use std::sync::Arc;

use async_trait::async_trait;

use super::wire::AskItem;
use super::{AskSettings, AsksHost, Person, Role};
use crate::db::DbHandle;
use crate::error::AppResult;
use crate::lidarr::Lidarr;
use crate::lidarr::request::{GetOutcome, RequestInput, RequestRecord};
use crate::lidarr::status::RequestState;
use crate::plex::state::{EventSink, PlexState};
use crate::social::Notifier;
use crate::social::sharing::Sharing;

pub struct LiveAsksHost {
    pub plex: PlexState,
    pub lidarr: Lidarr,
    pub sharing: Arc<Sharing>,
    pub db: DbHandle,
    pub events: Arc<dyn EventSink>,
    pub notifier: Arc<dyn Notifier>,
}

#[async_trait]
impl AsksHost for LiveAsksHost {
    async fn role(&self) -> Role {
        match self.plex.current_access().await {
            Some((machine_id, _, true)) => Role::Owner { machine_id },
            Some((machine_id, token, false)) => Role::Guest { machine_id, token },
            None => Role::Nobody,
        }
    }

    async fn people(&self) -> AppResult<Vec<Person>> {
        self.sharing.request_people().await
    }

    fn lidarr_usable(&self) -> bool {
        self.lidarr.status().usable()
    }

    async fn pick(&self, item: &AskItem) -> Result<String, String> {
        let picked = self
            .lidarr
            .pick(&item.artist, None, &item.title, item.song, item.isrc.as_deref())
            .await
            .map_err(|_| "Platter couldn't look this up on MusicBrainz just now. Request it again later.".to_string())?;
        match picked.group {
            Some(g) => Ok(g.id),
            None => Err(picked.reason.unwrap_or_else(|| "This isn't on MusicBrainz as an official release yet.".into())),
        }
    }

    async fn protected(&self, input: &RequestInput) -> Option<bool> {
        // Only an answer from a Lidarr that was reached counts.
        match self.lidarr.preview(input).await {
            Ok(p) if p.reachable => Some(p.protected),
            // A check that says no: Get says it again, with why.
            Err(e) if matches!(e.kind(), "invalid" | "notFound") => Some(false),
            _ => None,
        }
    }

    async fn get(&self, input: RequestInput) -> AppResult<GetOutcome> {
        self.lidarr.get(input).await
    }

    fn record(&self, release_group_id: &str) -> Option<RequestRecord> {
        let all = self.lidarr.requests();
        let mut mine = all.iter().filter(|r| r.release_group_id == release_group_id);
        mine.clone().find(|r| r.state != RequestState::Failed).or_else(|| mine.next_back()).cloned()
    }

    fn settings(&self) -> AskSettings {
        let s = crate::settings::load(&self.db);
        AskSettings {
            on: s.requests_from_others,
            rules: s.request_rules.into_iter().map(|r| (r.user_id, r.rule)).collect(),
            weekly_limit: s.request_weekly_limit,
            group_notices: s.request_group_notices,
        }
    }

    fn notify(&self, title: &str, body: &str) {
        self.notifier.notify(title, body);
    }

    fn emit(&self, event: &str, payload: serde_json::Value) {
        self.events.emit(event, payload);
    }

    fn refresh_library(&self) {
        if !self.db.sync_running()
            && let Err(e) = self.db.start_sync(false)
        {
            tracing::debug!("requests: library catch-up didn't start: {e}");
        }
    }

    fn now_ms(&self) -> u64 {
        crate::remote::peer::auth::now_ms()
    }
}
