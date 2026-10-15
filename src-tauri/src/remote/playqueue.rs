//! Server play queues: the list a player plays from after a move.
//!
//! `POST /playQueues?type=audio&uri=server://<machine>/com.plexapp.plugins.library/library/metadata/<ids>&key=/library/metadata/<start>`
//! creates one from Platter's queue order; `GET /playQueues/<id>` reads one
//! back (to resume here from wherever the player got to, and to show what a
//! player has coming up). Queue edits made here while a player plays go to
//! the same play queue: `PUT /playQueues/<id>?uri=...` adds,
//! `PUT /playQueues/<id>/items/<item>/move?after=<item>` moves and
//! `DELETE /playQueues/<id>/items/<item>` removes.

use serde_json::Value;

use super::service::ServerCtx;
use crate::audio::queue::Repeat;
use crate::error::{AppError, AppResult};
use crate::plex::client::{check_status, map_send_error};

pub const LIBRARY: &str = "com.plexapp.plugins.library";
/// Most songs sent in one play queue (keeps the request URL short).
pub const MAX_ITEMS: usize = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayQueue {
    pub id: String,
    pub selected_item_id: Option<String>,
}

fn s(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The `uri` for a list of library items on a server.
pub fn items_uri(machine_id: &str, ids: &[String]) -> String {
    format!("server://{machine_id}/{LIBRARY}/library/metadata/{}", ids.join(","))
}

/// Create a play queue holding `ids` (in order, at most [`MAX_ITEMS`]) with
/// `start` selected. Shuffle is off: the order already is the shuffled one.
pub async fn create(http: &reqwest::Client, server: &ServerCtx, ids: &[String], start: &str, repeat: Repeat) -> AppResult<PlayQueue> {
    let ids: Vec<String> = ids.iter().take(MAX_ITEMS).cloned().collect();
    if ids.is_empty() {
        return Err(AppError::Invalid("Nothing to play.".into()));
    }
    let url = server.url("/playQueues")?;
    let resp = http
        .post(url)
        .query(&[
            ("type", "audio".to_string()),
            ("uri", items_uri(&server.machine_id, &ids)),
            ("key", format!("/library/metadata/{start}")),
            ("shuffle", "0".into()),
            // Plex's create flag is on/off; repeat-one is set on the player.
            ("repeat", if repeat == Repeat::Off { "0" } else { "1" }.into()),
            ("continuous", "0".into()),
            ("own", "1".into()),
        ])
        .header("x-plex-token", server.token.header_value()?)
        .send()
        .await
        .map_err(map_send_error)?;
    let v: Value = check_status(resp).await?.json().await.map_err(map_send_error)?;
    let mc = &v["MediaContainer"];
    let id = s(&mc["playQueueID"]).ok_or_else(|| AppError::Decode("play queue without an id".into()))?;
    Ok(PlayQueue {
        id,
        selected_item_id: s(&mc["playQueueSelectedItemID"]),
    })
}

/// One entry of a play queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub item_id: String,
    pub key: String,
}

/// A play queue as the server has it now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub version: Option<String>,
    pub selected_item_id: Option<String>,
    /// Every entry the answer held, in play order.
    pub entries: Vec<Entry>,
    /// The server has more entries than the answer held.
    pub windowed: bool,
}

impl Snapshot {
    fn parse(v: &Value) -> Self {
        let mc = &v["MediaContainer"];
        let entries: Vec<Entry> = mc["Metadata"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| Some(Entry { item_id: s(&m["playQueueItemID"])?, key: s(&m["ratingKey"])? }))
            .collect();
        let total = s(&mc["playQueueTotalCount"]).and_then(|t| t.parse::<usize>().ok());
        Self {
            version: s(&mc["playQueueVersion"]),
            selected_item_id: s(&mc["playQueueSelectedItemID"]),
            windowed: total.is_some_and(|t| t > entries.len()),
            entries,
        }
    }

    /// Where the player is: its item id when the queue has it, else the
    /// server's selected item (when it's that song), else the first entry
    /// with `key`.
    pub fn position(&self, item_id: Option<&str>, key: Option<&str>) -> Option<usize> {
        let by_id = |id: &str| self.entries.iter().position(|e| e.item_id == id);
        item_id
            .and_then(by_id)
            .or_else(|| {
                self.selected_item_id
                    .as_deref()
                    .and_then(by_id)
                    .filter(|&i| key.is_none_or(|k| self.entries[i].key == k))
            })
            .or_else(|| key.and_then(|k| self.entries.iter().position(|e| e.key == k)))
    }
}

fn check_id(id: &str) -> AppResult<()> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::Invalid("bad play queue id".into()));
    }
    Ok(())
}

/// A play queue with its entries (up to twice [`MAX_ITEMS`] around the
/// selected one).
pub async fn read(http: &reqwest::Client, server: &ServerCtx, id: &str) -> AppResult<Snapshot> {
    check_id(id)?;
    let url = server.url(&format!("/playQueues/{id}"))?;
    let resp = http
        .get(url)
        .query(&[("window", (MAX_ITEMS * 2).to_string()), ("own", "1".into())])
        .header("x-plex-token", server.token.header_value()?)
        .send()
        .await
        .map_err(map_send_error)?;
    let v: Value = check_status(resp).await?.json().await.map_err(map_send_error)?;
    Ok(Snapshot::parse(&v))
}

/// Adds songs to a play queue (the server puts them after what's coming
/// up). Answers the queue after the change.
pub async fn add(http: &reqwest::Client, server: &ServerCtx, id: &str, ids: &[String]) -> AppResult<Snapshot> {
    check_id(id)?;
    let url = server.url(&format!("/playQueues/{id}"))?;
    let resp = http
        .put(url)
        .query(&[("uri", items_uri(&server.machine_id, ids)), ("next", "0".into())])
        .header("x-plex-token", server.token.header_value()?)
        .send()
        .await
        .map_err(map_send_error)?;
    let resp = check_status(resp).await?;
    let snap = resp.json::<Value>().await.ok().map(|v| Snapshot::parse(&v)).unwrap_or_default();
    if snap.entries.is_empty() { read(http, server, id).await } else { Ok(snap) }
}

/// Moves an entry to just after another one.
pub async fn move_after(http: &reqwest::Client, server: &ServerCtx, id: &str, item: &str, after: &str) -> AppResult<()> {
    check_id(id)?;
    check_id(item)?;
    check_id(after)?;
    let url = server.url(&format!("/playQueues/{id}/items/{item}/move"))?;
    let resp = http
        .put(url)
        .query(&[("after", after)])
        .header("x-plex-token", server.token.header_value()?)
        .send()
        .await
        .map_err(map_send_error)?;
    check_status(resp).await.map(|_| ())
}

/// Removes an entry.
pub async fn remove(http: &reqwest::Client, server: &ServerCtx, id: &str, item: &str) -> AppResult<()> {
    check_id(id)?;
    check_id(item)?;
    let url = server.url(&format!("/playQueues/{id}/items/{item}"))?;
    let resp = http
        .delete(url)
        .header("x-plex-token", server.token.header_value()?)
        .send()
        .await
        .map_err(map_send_error)?;
    check_status(resp).await.map(|_| ())
}

/// Makes what comes after the entry at `at` exactly `want` (rating keys, in
/// order) with as few requests as it can: entries already in place stay,
/// ones out of place are moved, missing songs are added and the rest are
/// removed. Entries before `at` (already played) are left alone.
pub async fn set_upcoming(
    http: &reqwest::Client,
    server: &ServerCtx,
    id: &str,
    snap: &Snapshot,
    at: usize,
    want: &[String],
) -> AppResult<()> {
    let Some(cur) = snap.entries.get(at) else {
        return Err(AppError::Invalid("the player's song isn't in its play queue".into()));
    };
    let mut prev = cur.item_id.clone();
    // First the songs that aren't wanted any more (as many as are extra).
    let mut need: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for k in want {
        *need.entry(k.as_str()).or_default() += 1;
    }
    let mut tail: Vec<Entry> = Vec::new();
    for e in &snap.entries[at + 1..] {
        match need.get_mut(e.key.as_str()).filter(|n| **n > 0) {
            Some(n) => {
                *n -= 1;
                tail.push(e.clone());
            }
            None => remove(http, server, id, &e.item_id).await?,
        }
    }
    let mut known: Vec<String> = snap.entries.iter().map(|e| e.item_id.clone()).collect();
    let mut i = 0;
    while i < want.len() {
        let key = &want[i];
        if tail.get(i).is_some_and(|e| &e.key == key) {
            prev = tail[i].item_id.clone();
            i += 1;
            continue;
        }
        if let Some(j) = (i + 1..tail.len()).find(|&j| &tail[j].key == key) {
            let e = tail.remove(j);
            move_after(http, server, id, &e.item_id, &prev).await?;
            prev = e.item_id.clone();
            tail.insert(i, e);
            i += 1;
            continue;
        }
        // Missing: this song and the missing ones right after it, in one add.
        let mut run = vec![key.clone()];
        while let Some(k) = want.get(i + run.len()) {
            if tail.iter().skip(i).any(|e| &e.key == k) {
                break;
            }
            run.push(k.clone());
        }
        let after = add(http, server, id, &run).await?;
        let new: Vec<Entry> = after.entries.iter().filter(|e| !known.contains(&e.item_id)).cloned().collect();
        if new.len() != run.len() {
            return Err(AppError::Decode("the play queue didn't take the songs".into()));
        }
        known.extend(new.iter().map(|e| e.item_id.clone()));
        // Moved only when the server didn't put them right after `prev`.
        let in_place = after
            .entries
            .iter()
            .position(|e| e.item_id == prev)
            .and_then(|p| after.entries.get(p + 1..p + 1 + new.len()))
            .is_some_and(|s| s == new.as_slice());
        for e in &new {
            if !in_place {
                move_after(http, server, id, &e.item_id, &prev).await?;
            }
            prev = e.item_id.clone();
        }
        for (k, e) in new.into_iter().enumerate() {
            tail.insert(i + k, e);
        }
        i += run.len();
    }
    for e in tail.drain(want.len().min(tail.len())..) {
        remove(http, server, id, &e.item_id).await?;
    }
    Ok(())
}

/// Rating keys of a play queue's items, in order.
pub async fn items(http: &reqwest::Client, server: &ServerCtx, id: &str) -> AppResult<Vec<String>> {
    if !id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(AppError::Invalid("bad play queue id".into()));
    }
    let url = server.url(&format!("/playQueues/{id}"))?;
    let resp = http
        .get(url)
        .header("x-plex-token", server.token.header_value()?)
        .send()
        .await
        .map_err(map_send_error)?;
    let v: Value = check_status(resp).await?.json().await.map_err(map_send_error)?;
    Ok(v["MediaContainer"]["Metadata"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| s(&m["ratingKey"]))
        .collect())
}

/// The play queue id in a `containerKey` such as `/playQueues/12?own=1`.
pub fn id_from_container_key(key: &str) -> Option<String> {
    let rest = key.split('?').next()?.strip_prefix("/playQueues/")?;
    let id = rest.trim_end_matches('/');
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_and_container_keys() {
        assert_eq!(
            items_uri("abc", &["1".into(), "2".into()]),
            "server://abc/com.plexapp.plugins.library/library/metadata/1,2"
        );
        assert_eq!(id_from_container_key("/playQueues/12?window=100&own=1").as_deref(), Some("12"));
        assert_eq!(id_from_container_key("/playQueues/12").as_deref(), Some("12"));
        assert_eq!(id_from_container_key("/library/metadata/1"), None);
        assert_eq!(id_from_container_key("/playQueues/../x"), None);
    }
}
