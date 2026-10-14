//! Server play queues: the list a player plays from after a move.
//!
//! `POST /playQueues?type=audio&uri=server://<machine>/com.plexapp.plugins.library/library/metadata/<ids>&key=/library/metadata/<start>`
//! creates one from Platter's queue order; `GET /playQueues/<id>` reads one
//! back (to resume here from wherever the player got to).

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
