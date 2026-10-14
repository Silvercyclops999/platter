//! `now_playing_details`: the extras the now-playing panel shows for a track
//! that the library cache doesn't keep. The album artist's bio and photo,
//! extra credits (composer, writer, lyricist, producer) and lyrics when the
//! track has a lyrics stream.
//!
//! Everything here is optional. Plex leaves most of it out for most tracks,
//! so parsing is lenient and a failed side request (artist, lyrics) just
//! leaves that part empty instead of failing the whole call.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use super::client::PlexClient;
use super::lenient;
use super::state::PlexState;
use crate::error::{AppError, AppResult};

/// Longest lyrics kept (lines). Real songs are far shorter.
const MAX_LYRIC_LINES: usize = 400;
/// Plex stream type for lyrics.
const LYRICS_STREAM: i64 = 4;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowPlayingDetails {
    pub track_id: String,
    pub artist: Option<ArtistAbout>,
    /// People credited beyond the track's artists, in Plex's order.
    pub credits: Vec<Contributor>,
    pub lyrics: Option<Lyrics>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtistAbout {
    pub id: String,
    pub title: String,
    /// The artist bio, trimmed. None when Plex has none.
    pub summary: Option<String>,
    pub thumb: Option<String>,
    /// Background art, when the artist has it.
    pub art: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contributor {
    pub name: String,
    /// "composer", "writer", "lyricist" or "producer".
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Lyrics {
    /// The source had timestamps (shown as plain text for now).
    pub synced: bool,
    /// Lines without timestamps; "" marks a break between verses.
    pub lines: Vec<String>,
}

// ---------------------------------------------------------------- wire shapes

#[derive(Debug, Default, Deserialize)]
struct Envelope {
    #[serde(default, rename = "MediaContainer")]
    media_container: Container,
}

#[derive(Debug, Default, Deserialize)]
struct Container {
    #[serde(default, rename = "Metadata")]
    metadata: Vec<Meta>,
}

#[derive(Debug, Default, Deserialize)]
struct TagItem {
    #[serde(default, deserialize_with = "lenient::string")]
    tag: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    #[serde(default, deserialize_with = "lenient::string")]
    rating_key: String,
    #[serde(default, deserialize_with = "lenient::string")]
    title: String,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    grandparent_rating_key: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    summary: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    thumb: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    art: Option<String>,
    #[serde(default, rename = "Composer")]
    composer: Vec<TagItem>,
    #[serde(default, rename = "Writer")]
    writer: Vec<TagItem>,
    #[serde(default, rename = "Lyricist")]
    lyricist: Vec<TagItem>,
    #[serde(default, rename = "Producer")]
    producer: Vec<TagItem>,
    #[serde(default, rename = "Media")]
    media: Vec<Media>,
}

#[derive(Debug, Default, Deserialize)]
struct Media {
    #[serde(default, rename = "Part")]
    parts: Vec<Part>,
}

#[derive(Debug, Default, Deserialize)]
struct Part {
    #[serde(default, rename = "Stream")]
    streams: Vec<Stream>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stream {
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    id: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_i64")]
    stream_type: Option<i64>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    key: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    codec: Option<String>,
    #[serde(default, deserialize_with = "lenient::opt_string")]
    format: Option<String>,
}

fn first_meta(body: &[u8]) -> AppResult<Meta> {
    let env: Envelope = serde_json::from_slice(body)
        .map_err(|e| AppError::Decode(format!("/library/metadata: {e}")))?;
    env.media_container
        .metadata
        .into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("item".into()))
}

// ---------------------------------------------------------------- parsing

/// What the track's own metadata tells us.
#[derive(Debug, Default, PartialEq)]
struct TrackExtras {
    artist_id: Option<String>,
    credits: Vec<Contributor>,
    /// Path of the lyrics stream, if the track has one.
    lyrics_path: Option<String>,
}

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn track_extras(m: Meta) -> TrackExtras {
    let mut credits: Vec<Contributor> = Vec::new();
    for (role, tags) in [
        ("composer", &m.composer),
        ("writer", &m.writer),
        ("lyricist", &m.lyricist),
        ("producer", &m.producer),
    ] {
        for t in tags {
            let name = t.tag.trim();
            if name.is_empty() {
                continue;
            }
            match credits
                .iter_mut()
                .find(|c| c.name.eq_ignore_ascii_case(name))
            {
                Some(c) if !c.roles.iter().any(|r| r == role) => c.roles.push(role.into()),
                Some(_) => {}
                None => credits.push(Contributor {
                    name: name.to_owned(),
                    roles: vec![role.into()],
                }),
            }
        }
    }
    let lyric_streams: Vec<&Stream> = m
        .media
        .iter()
        .flat_map(|md| &md.parts)
        .flat_map(|p| &p.streams)
        .filter(|s| s.stream_type == Some(LYRICS_STREAM))
        .collect();
    // Prefer a timed stream (lrc) over plain text when there are both.
    let is_lrc = |s: &&Stream| {
        [&s.codec, &s.format]
            .iter()
            .any(|v| v.as_deref().is_some_and(|c| c.eq_ignore_ascii_case("lrc")))
    };
    let lyrics_path = lyric_streams
        .iter()
        .find(|s| is_lrc(s))
        .or(lyric_streams.first())
        .and_then(|s| match (&s.key, s.id) {
            (Some(k), _) if k.starts_with('/') => Some(k.clone()),
            (_, Some(id)) => Some(format!("/library/streams/{id}")),
            _ => None,
        });
    TrackExtras {
        artist_id: non_empty(m.grandparent_rating_key),
        credits,
        lyrics_path,
    }
}

fn artist_about(m: Meta) -> ArtistAbout {
    ArtistAbout {
        id: m.rating_key,
        title: m.title,
        summary: non_empty(m.summary),
        thumb: non_empty(m.thumb),
        art: non_empty(m.art),
    }
}

/// True for an LRC time tag body such as `01:23.45` or `1:02`.
fn is_time_tag(s: &str) -> bool {
    let mut parts = s.splitn(2, ':');
    let (Some(min), Some(rest)) = (parts.next(), parts.next()) else {
        return false;
    };
    !min.is_empty()
        && min.chars().all(|c| c.is_ascii_digit())
        && !rest.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ':')
}

/// Remove `<mm:ss.xx>` word timings from an enhanced LRC line.
fn strip_word_times(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find('<') {
        match rest[start..].find('>') {
            Some(end) if is_time_tag(&rest[start + 1..start + end]) => {
                out.push_str(&rest[..start]);
                rest = &rest[start + end + 1..];
            }
            _ => {
                out.push_str(&rest[..=start]);
                rest = &rest[start + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Tidy lines: collapse runs of blank lines, trim blanks at both ends, cap.
fn tidy(lines: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in lines {
        if l.is_empty() && out.last().is_none_or(|p| p.is_empty()) {
            continue;
        }
        out.push(l);
        if out.len() >= MAX_LYRIC_LINES {
            break;
        }
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Plain or LRC lyrics text to lines. Time tags are dropped (and mark the
/// lyrics as synced); LRC header tags such as `[ar:...]` are left out.
pub fn parse_lyrics_text(text: &str) -> Lyrics {
    let mut synced = false;
    let mut lines = Vec::new();
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let mut line = raw.trim();
        let mut header = false;
        while let Some(rest) = line.strip_prefix('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[..end];
            if is_time_tag(tag) {
                synced = true;
            } else if tag.contains(':') {
                header = true;
            } else {
                break;
            }
            line = rest[end + 1..].trim_start();
        }
        if header && line.is_empty() {
            continue;
        }
        lines.push(strip_word_times(line).trim().to_owned());
    }
    Lyrics {
        synced,
        lines: tidy(lines),
    }
}

/// Lyrics in Plex's JSON form: `Lyrics[].Line[]`, each line a list of
/// `Span` with `text` (or a `text` of its own), timed lines carry
/// `startOffset`.
fn parse_lyrics_json(v: &Value) -> Option<Lyrics> {
    let lyrics = v.get("MediaContainer")?.get("Lyrics")?.as_array()?;
    let mut synced = false;
    let mut lines = Vec::new();
    for l in lyrics.iter().filter_map(|l| l.get("Line")?.as_array()) {
        for line in l {
            if line.get("startOffset").is_some() {
                synced = true;
            }
            let text = match line.get("Span").and_then(Value::as_array) {
                Some(spans) => spans
                    .iter()
                    .filter_map(|s| s.get("text")?.as_str())
                    .collect::<String>(),
                None => line
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            };
            lines.push(text.trim().to_owned());
        }
    }
    Some(Lyrics {
        synced,
        lines: tidy(lines),
    })
}

/// A lyrics stream body: JSON when the server sends its structured form,
/// otherwise LRC or plain text. None when there are no words in it.
pub fn parse_lyrics(body: &[u8]) -> Option<Lyrics> {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    let lyrics = if trimmed.starts_with('{') {
        serde_json::from_str::<Value>(trimmed)
            .ok()
            .and_then(|v| parse_lyrics_json(&v))?
    } else if trimmed.starts_with('<') {
        // XML would mean the server ignored the JSON accept header.
        return None;
    } else {
        parse_lyrics_text(trimmed)
    };
    (!lyrics.lines.is_empty()).then_some(lyrics)
}

// ---------------------------------------------------------------- fetching

/// Plex ids are numbers; anything else would change the request path.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

/// Fetch and assemble the details for a track. Only the track's own
/// metadata is required; the artist and lyrics requests may fail quietly.
pub async fn fetch(client: &PlexClient, track_id: &str) -> AppResult<NowPlayingDetails> {
    if !valid_id(track_id) {
        return Err(AppError::Invalid("track id".into()));
    }
    let body = client
        .get_bytes(&format!("/library/metadata/{track_id}"), &[])
        .await?;
    let extras = track_extras(first_meta(&body)?);

    let artist = async {
        let id = extras.artist_id.as_deref().filter(|id| valid_id(id))?;
        let body = client
            .get_bytes(&format!("/library/metadata/{id}"), &[])
            .await
            .map_err(|e| tracing::debug!(kind = e.kind(), "artist details unavailable"))
            .ok()?;
        first_meta(&body).ok().map(artist_about)
    };
    let lyrics = async {
        let path = extras.lyrics_path.as_deref()?;
        let body = client
            .get_bytes(path, &[])
            .await
            .map_err(|e| tracing::debug!(kind = e.kind(), "lyrics unavailable"))
            .ok()?;
        parse_lyrics(&body)
    };
    let (artist, lyrics) = futures::join!(artist, lyrics);
    Ok(NowPlayingDetails {
        track_id: track_id.to_owned(),
        artist,
        credits: extras.credits,
        lyrics,
    })
}

/// docs/IPC.md `now_playing_details`.
#[tauri::command]
pub async fn now_playing_details(
    state: State<'_, PlexState>,
    track_id: String,
) -> AppResult<NowPlayingDetails> {
    let client = state.client().await?;
    fetch(&client, &track_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plex::Token;
    use crate::plex::identity::ClientIdentity;
    use serde_json::json;
    use std::sync::Arc;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn meta(v: Value) -> Meta {
        first_meta(&serde_json::to_vec(&json!({"MediaContainer": {"Metadata": [v]}})).unwrap())
            .unwrap()
    }

    /// A track as Plex returns it from `/library/metadata/{id}`, with the
    /// optional credit tags and a lyrics stream next to the audio stream.
    fn track_fixture() -> Value {
        json!({
            "ratingKey": "500", "type": "track", "title": "Lanterns",
            "grandparentRatingKey": 100, "grandparentTitle": "Odessa Wren",
            "Composer": [{"tag": "Odessa Wren"}, {"tag": "M. Hale"}],
            "Writer": [{"tag": "m. hale"}, {"tag": " "}],
            "Producer": [{"tag": "June Park"}],
            "Media": [{"Part": [{"Stream": [
                {"id": 1, "streamType": 2, "codec": "flac"},
                {"id": "77", "streamType": "4", "codec": "txt", "key": "/library/streams/77"},
                {"id": 78, "streamType": 4, "format": "lrc"}
            ]}]}]
        })
    }

    #[test]
    fn track_extras_merge_roles_and_prefer_timed_lyrics() {
        let x = track_extras(meta(track_fixture()));
        assert_eq!(x.artist_id.as_deref(), Some("100"));
        assert_eq!(
            x.credits,
            vec![
                Contributor {
                    name: "Odessa Wren".into(),
                    roles: vec!["composer".into()]
                },
                Contributor {
                    name: "M. Hale".into(),
                    roles: vec!["composer".into(), "writer".into()]
                },
                Contributor {
                    name: "June Park".into(),
                    roles: vec!["producer".into()]
                },
            ]
        );
        // The lrc stream has no key, so its path is built from the id.
        assert_eq!(x.lyrics_path.as_deref(), Some("/library/streams/78"));
    }

    #[test]
    fn bare_track_has_no_extras() {
        let x = track_extras(meta(json!({"ratingKey": "1", "title": "t"})));
        assert_eq!(x, TrackExtras::default());
    }

    #[test]
    fn artist_about_trims_and_drops_empty_fields() {
        let a = artist_about(meta(json!({
            "ratingKey": 100, "title": "Odessa Wren", "summary": "  Born by the sea.\n ",
            "thumb": "/library/metadata/100/thumb/1", "art": ""
        })));
        assert_eq!(a.id, "100");
        assert_eq!(a.summary.as_deref(), Some("Born by the sea."));
        assert_eq!(a.thumb.as_deref(), Some("/library/metadata/100/thumb/1"));
        assert_eq!(a.art, None);
        let empty = artist_about(meta(
            json!({"ratingKey": "2", "title": "X", "summary": "   "}),
        ));
        assert_eq!(empty.summary, None);
    }

    #[test]
    fn lrc_text_drops_tags_and_marks_synced() {
        let l = parse_lyrics(
            "\u{feff}[ar:Odessa Wren]\n[ti:Lanterns]\n[offset:+100]\n\n[00:12.30]First <00:12.80>line\n[00:15.00][01:15.00]Chorus line\n[00:20.00]\n\n\n[00:25.10]Last line\n\n"
                .as_bytes(),
        )
        .unwrap();
        assert!(l.synced);
        assert_eq!(l.lines, vec!["First line", "Chorus line", "", "Last line"]);
    }

    #[test]
    fn plain_text_keeps_verse_breaks() {
        let l = parse_lyrics(b"One\r\nTwo\r\n\r\nThree [softly]\n").unwrap();
        assert!(!l.synced);
        assert_eq!(l.lines, vec!["One", "Two", "", "Three [softly]"]);
    }

    #[test]
    fn structured_lyrics_join_spans() {
        let body = json!({"MediaContainer": {"Lyrics": [{"Line": [
            {"startOffset": 1000, "Span": [{"text": "Hello "}, {"text": "there"}]},
            {"Span": []},
            {"text": "Plain line"}
        ]}]}});
        let l = parse_lyrics(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert!(l.synced);
        assert_eq!(l.lines, vec!["Hello there", "", "Plain line"]);
    }

    #[test]
    fn empty_or_xml_lyrics_are_none() {
        assert_eq!(parse_lyrics(b"   \n\n"), None);
        assert_eq!(parse_lyrics(b"[ar:Someone]\n"), None);
        assert_eq!(parse_lyrics(b"<MediaContainer/>"), None);
        assert_eq!(parse_lyrics(b"{\"MediaContainer\":{}}"), None);
    }

    #[test]
    fn long_lyrics_are_capped() {
        let text: String = (0..1000).map(|i| format!("line {i}\n")).collect();
        assert_eq!(
            parse_lyrics(text.as_bytes()).unwrap().lines.len(),
            MAX_LYRIC_LINES
        );
    }

    #[test]
    fn ids_must_be_numbers() {
        assert!(valid_id("500"));
        assert!(!valid_id(""));
        assert!(!valid_id("../500"));
        assert!(!valid_id("5?x=1"));
    }

    fn client(server: &MockServer) -> PlexClient {
        PlexClient::new(
            &server.uri(),
            Token::new("details-test-token").unwrap(),
            Arc::new(ClientIdentity::with_id("cid")),
            None,
        )
        .unwrap()
    }

    async fn reply(server: &MockServer, p: &str, r: ResponseTemplate) {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(r)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn fetch_assembles_track_artist_and_lyrics() {
        let server = MockServer::start().await;
        let wrap = |v: Value| json!({"MediaContainer": {"Metadata": [v]}});
        reply(
            &server,
            "/library/metadata/500",
            ResponseTemplate::new(200).set_body_json(wrap(track_fixture())),
        )
        .await;
        reply(
            &server,
            "/library/metadata/100",
            ResponseTemplate::new(200).set_body_json(wrap(
                json!({"ratingKey": "100", "title": "Odessa Wren", "summary": "Bio."}),
            )),
        )
        .await;
        reply(
            &server,
            "/library/streams/78",
            ResponseTemplate::new(200).set_body_string("[00:01.00]Hi\n[00:02.00]There"),
        )
        .await;
        let d = fetch(&client(&server), "500").await.unwrap();
        assert_eq!(d.track_id, "500");
        assert_eq!(d.artist.unwrap().summary.as_deref(), Some("Bio."));
        assert_eq!(d.credits.len(), 3);
        assert_eq!(d.lyrics.unwrap().lines, vec!["Hi", "There"]);
    }

    #[tokio::test]
    async fn side_requests_fail_quietly() {
        let server = MockServer::start().await;
        reply(
            &server,
            "/library/metadata/500",
            ResponseTemplate::new(200)
                .set_body_json(json!({"MediaContainer": {"Metadata": [track_fixture()]}})),
        )
        .await;
        reply(&server, "/library/metadata/100", ResponseTemplate::new(500)).await;
        reply(&server, "/library/streams/78", ResponseTemplate::new(404)).await;
        let d = fetch(&client(&server), "500").await.unwrap();
        assert_eq!(d.artist, None);
        assert_eq!(d.lyrics, None);
        assert_eq!(d.credits.len(), 3);
    }

    #[tokio::test]
    async fn missing_track_and_bad_ids_are_errors() {
        let server = MockServer::start().await;
        reply(&server, "/library/metadata/9", ResponseTemplate::new(404)).await;
        let c = client(&server);
        assert_eq!(fetch(&c, "9").await.unwrap_err().kind(), "notFound");
        assert_eq!(fetch(&c, "../9").await.unwrap_err().kind(), "invalid");
    }

    #[test]
    fn serializes_camel_case() {
        let d = NowPlayingDetails {
            track_id: "1".into(),
            artist: None,
            credits: vec![],
            lyrics: Some(Lyrics {
                synced: false,
                lines: vec!["a".into()],
            }),
        };
        let v = serde_json::to_value(d).unwrap();
        assert_eq!(v["trackId"], "1");
        assert!(v["artist"].is_null());
        assert_eq!(v["lyrics"]["lines"][0], "a");
    }
}
