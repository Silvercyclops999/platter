//! What a Cast device is sent: the server play queue for Plex's receiver,
//! and the same songs as stream URLs for the default receiver. Both carry a
//! token the device needs to reach the server; Platter asks the server for
//! a transient one for this. URLs built here are never logged.

use url::Url;

use super::link::{CastPlan, DirectItem, PlexLoad};
use crate::audio::queue::Repeat;
use crate::model::Track;
use crate::plex::Token;
use crate::remote::service::ServerCtx;

/// Songs sent to the default receiver at most (it holds the whole queue).
pub const MAX_ITEMS: usize = 100;

/// The server's transient token for handing to another device, from
/// `GET /security/token?type=delegation&scope=all`. None when the server
/// doesn't give one.
pub async fn transient_token(http: &reqwest::Client, server: &ServerCtx) -> Option<Token> {
    let mut u = server.url("/security/token").ok()?;
    u.set_query(Some("type=delegation&scope=all"));
    let resp = http
        .get(u)
        .header("x-plex-token", server.token.header_value().ok()?)
        .header("accept", "application/json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body = resp.text().await.ok()?;
    token_from(&body)
}

/// The token in a `/security/token` answer (JSON or XML).
pub fn token_from(body: &str) -> Option<Token> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        return v["MediaContainer"]["token"].as_str().and_then(|t| Token::new(t).ok());
    }
    let at = body.find("token=\"")? + 7;
    let end = body[at..].find('"')? + at;
    Token::new(&body[at..end]).ok()
}

/// Codecs Cast devices play from the original file.
fn cast_type(t: &Track) -> Option<&'static str> {
    let codec = t.codec.as_deref().unwrap_or("").to_ascii_lowercase();
    let container = t.container.as_deref().unwrap_or("").to_ascii_lowercase();
    match (codec.as_str(), container.as_str()) {
        ("flac", _) => Some("audio/flac"),
        ("mp3", _) => Some("audio/mpeg"),
        ("aac", "mp4" | "m4a" | "") => Some("audio/mp4"),
        ("aac", "aac") => Some("audio/aac"),
        ("opus" | "vorbis", "ogg") => Some("audio/ogg"),
        ("opus", "webm") => Some("audio/webm"),
        ("pcm", "wav") => Some("audio/wav"),
        _ => None,
    }
}

fn with_token(mut u: Url, token: &Token) -> String {
    u.query_pairs_mut().append_pair("X-Plex-Token", token.expose());
    u.to_string()
}

fn direct(server: &ServerCtx, token: &Token, client_id: &str, t: &Track) -> Option<DirectItem> {
    // Ids carry their server's tag (crate::sid); the server wants its own.
    let key = crate::sid::key(&t.id);
    let part = crate::sid::key(&t.part_key);
    let (url, content_type) = match cast_type(t) {
        Some(ct) if part.starts_with('/') => (with_token(server.url(part).ok()?, token), ct),
        // Anything else goes through the server's transcoder as MP3.
        _ => {
            let mut u = server.url("/music/:/transcode/universal/start.mp3").ok()?;
            u.query_pairs_mut()
                .append_pair("path", &format!("/library/metadata/{key}"))
                .append_pair("mediaIndex", "0")
                .append_pair("partIndex", "0")
                .append_pair("protocol", "http")
                .append_pair("directPlay", "0")
                .append_pair("directStream", "0")
                .append_pair("musicBitrate", "320")
                .append_pair("session", &uuid::Uuid::new_v4().to_string())
                .append_pair("X-Plex-Client-Identifier", client_id);
            (with_token(u, token), "audio/mpeg")
        }
    };
    let image = t
        .thumb
        .as_deref()
        .map(crate::sid::key)
        .filter(|p| p.starts_with('/'))
        .and_then(|p| server.url(p).ok())
        .map(|u| with_token(u, token));
    Some(DirectItem {
        rating_key: key.to_string(),
        url,
        content_type: content_type.to_string(),
        title: t.title.clone(),
        artist: t.track_artist.clone().unwrap_or_else(|| t.artist_title.clone()),
        album: t.album_title.clone(),
        image,
        duration_ms: t.duration_ms,
    })
}

/// Whether a song is on `server`: songs from another of the account's
/// servers can't be played through this one.
fn on_server(server: &ServerCtx, t: &Track) -> bool {
    crate::sid::tag_of(&t.id).is_none_or(|tag| tag == crate::sid::tag(&server.machine_id))
}

/// The plan for `tracks` (the current song first), starting at `offset_ms`.
/// Songs on another server are left out.
pub fn plan(
    server: &ServerCtx,
    token: &Token,
    client_id: &str,
    play_queue_id: &str,
    tracks: &[Track],
    offset_ms: u64,
    repeat: Repeat,
) -> CastPlan {
    let plex = tracks.first().map(|first| PlexLoad {
        rating_key: crate::sid::key(&first.id).to_string(),
        offset_ms,
        machine_id: server.machine_id.clone(),
        protocol: server.base.scheme().to_string(),
        address: server.base.host_str().unwrap_or_default().to_string(),
        port: server.base.port_or_known_default().unwrap_or(32400),
        token: token.expose().to_string(),
        play_queue_id: play_queue_id.to_string(),
    });
    CastPlan {
        plex,
        items: tracks
            .iter()
            .filter(|t| on_server(server, t))
            .take(MAX_ITEMS)
            .filter_map(|t| direct(server, token, client_id, t))
            .collect(),
        start: 0,
        offset_ms,
        repeat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::testutil::track;

    fn server() -> ServerCtx {
        ServerCtx {
            base: Url::parse("https://192-0-2-1.abc.plex.direct:32400").unwrap(),
            token: Token::new("server-token").unwrap(),
            machine_id: "srv".into(),
        }
    }

    #[test]
    fn transient_tokens_parse_from_json_or_xml() {
        assert_eq!(token_from(r#"{"MediaContainer":{"token":"transient-1"}}"#).unwrap().expose(), "transient-1");
        assert_eq!(token_from(r#"<MediaContainer size="0" token="transient-2"/>"#).unwrap().expose(), "transient-2");
        assert!(token_from("{}").is_none());
        assert!(token_from("nothing").is_none());
    }

    #[test]
    fn plan_uses_the_given_token_and_the_play_queue() {
        let mut a = track("1", "A", "/library/parts/1/2/file.flac");
        a.codec = Some("flac".into());
        let mut b = track("2", "A", "/library/parts/3/4/file.m4a");
        b.codec = Some("alac".into());
        b.container = Some("mp4".into());
        let tok = Token::new("transient").unwrap();
        let p = plan(&server(), &tok, "me", "55", &[a, b], 12_000, Repeat::All);
        let plex = p.plex.unwrap();
        assert_eq!((plex.rating_key.as_str(), plex.play_queue_id.as_str(), plex.port), ("1", "55", 32400));
        assert_eq!(plex.address, "192-0-2-1.abc.plex.direct");
        assert_eq!(plex.token, "transient");
        assert_eq!(p.items.len(), 2);
        assert!(p.items[0].url.contains("/library/parts/1/2/file.flac") && p.items[0].url.contains("X-Plex-Token=transient"));
        assert_eq!(p.items[0].content_type, "audio/flac");
        // ALAC isn't something Cast devices play: it's transcoded.
        assert!(p.items[1].url.contains("/music/:/transcode/universal/start.mp3"));
        assert_eq!(p.items[1].content_type, "audio/mpeg");
        assert!(!p.items.iter().any(|i| i.url.contains("server-token")));
        assert_eq!((p.offset_ms, p.repeat), (12_000, Repeat::All));
    }

    #[test]
    fn tagged_ids_reach_the_server_as_its_own_and_other_servers_songs_stay_out() {
        let tag = crate::sid::tag("srv");
        let other = crate::sid::tag("elsewhere");
        let mut a = track(&format!("{tag}:41"), "A", "/library/parts/1/2/file.m4a");
        a.codec = Some("alac".into());
        a.thumb = Some(format!("{tag}:/library/metadata/40/thumb/9"));
        let mut b = track(&format!("{other}:42"), "A", "/library/parts/5/6/file.flac");
        b.codec = Some("flac".into());
        let mut c = track(&format!("{tag}:43"), "A", "/library/parts/7/8/file.flac");
        c.codec = Some("flac".into());
        let tok = Token::new("transient").unwrap();
        let p = plan(&server(), &tok, "me", "55", &[a, b, c], 0, Repeat::Off);
        assert_eq!(p.plex.unwrap().rating_key, "41");
        assert_eq!(p.items.len(), 2, "the other server's song is left out");
        let u = Url::parse(&p.items[0].url).unwrap();
        let path = u.query_pairs().find(|(k, _)| k == "path").unwrap().1.into_owned();
        assert_eq!(path, "/library/metadata/41");
        assert_eq!(p.items[0].rating_key, "41");
        let img = Url::parse(p.items[0].image.as_deref().unwrap()).unwrap();
        assert_eq!(img.path(), "/library/metadata/40/thumb/9");
        assert_eq!(p.items[1].rating_key, "43");
        assert_eq!(Url::parse(&p.items[1].url).unwrap().path(), "/library/parts/7/8/file.flac");
        assert!(!p.items.iter().any(|i| i.url.contains(&tag) || i.url.contains(&other)));
    }
}
