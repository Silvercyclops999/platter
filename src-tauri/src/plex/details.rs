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
    /// The source had timestamps; `timed` holds them.
    pub synced: bool,
    /// Lines without timestamps, in the file's order; "" marks a break
    /// between verses.
    pub lines: Vec<String>,
    /// Timed lines in play order (a line with several time tags appears once
    /// per tag). Empty when the lyrics aren't synced. A line with empty text
    /// is an instrumental gap.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub timed: Vec<LyricLine>,
    /// Who wrote and who supplied the lyrics, shown under them (licensed
    /// lyrics ask for this). None when the source names neither.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit: Option<String>,
    /// Where the lyrics came from.
    pub source: LyricsSource,
}

/// Where lyrics came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LyricsSource {
    /// Plex's own lyrics (an agent such as LyricFind supplied them).
    #[default]
    Plex,
    /// A .lrc or .txt file next to the song, served by Plex.
    File,
    /// Written by the user in the app.
    User,
    /// LRCLIB, when its setting is on and Plex had none.
    Lrclib,
    /// Transcribed on this computer and not edited yet (may be wrong).
    Auto,
}

/// One timed line.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricLine {
    pub start_ms: u32,
    pub text: String,
    /// Word timing (enhanced LRC), joined they make `text`. Empty when the
    /// source times whole lines only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub words: Vec<LyricWord>,
}

/// One timed word (with its trailing space, if any).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LyricWord {
    pub start_ms: u32,
    pub text: String,
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
    /// The agent that supplied a lyrics stream.
    #[serde(default, deserialize_with = "lenient::opt_string")]
    provider: Option<String>,
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
    /// That stream is a .lrc or .txt file next to the song, not lyrics an
    /// agent supplied.
    lyrics_file: bool,
}

/// A lyrics stream comes from a file next to the song when Plex's local
/// media agent supplied it (or no agent is named). Lyrics services name
/// their own agent (`com.plexapp.agents.lyricfind`).
fn is_sidecar(provider: Option<&str>) -> bool {
    match provider.map(str::trim).filter(|p| !p.is_empty()) {
        None => true,
        Some(p) => p.to_ascii_lowercase().contains("localmedia"),
    }
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
    let chosen = lyric_streams.iter().find(|s| is_lrc(s)).or(lyric_streams.first());
    let lyrics_path = chosen.and_then(|s| match (&s.key, s.id) {
        (Some(k), _) if k.starts_with('/') => Some(k.clone()),
        (_, Some(id)) => Some(format!("/library/streams/{id}")),
        _ => None,
    });
    TrackExtras {
        artist_id: non_empty(m.grandparent_rating_key),
        credits,
        lyrics_file: lyrics_path.is_some() && chosen.is_some_and(|s| is_sidecar(s.provider.as_deref())),
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

/// An LRC time tag body (`01:23.45`, `1:02`, `01:23.456`, `01:23:45`) in ms.
pub fn time_ms(s: &str) -> Option<u32> {
    let (min, rest) = s.split_once(':')?;
    if min.is_empty() || min.len() > 4 || !min.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (sec, frac) = match rest.find(['.', ':']) {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    if sec.is_empty() || sec.len() > 2 || !sec.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !frac.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let frac_ms = match frac.len() {
        0 => 0,
        1 => frac.parse::<u32>().ok()? * 100,
        2 => frac.parse::<u32>().ok()? * 10,
        _ => frac[..3].parse::<u32>().ok()?,
    };
    let min: u32 = min.parse().ok()?;
    let sec: u32 = sec.parse().ok()?;
    Some(min * 60_000 + sec * 1000 + frac_ms)
}

/// Split an enhanced LRC line into its plain text and its timed words
/// (`<mm:ss.xx>` tags). Words are empty when the line has no word tags; the
/// first word has no time of its own when text comes before the first tag.
fn split_words(line: &str) -> (String, Vec<(Option<u32>, String)>) {
    let mut segs: Vec<(Option<u32>, String)> = vec![(None, String::new())];
    let mut rest = line;
    let mut any = false;
    while let Some(start) = rest.find('<') {
        let tag = rest[start..].find('>').and_then(|end| Some((end, time_ms(&rest[start + 1..start + end])?)));
        let last = segs.last_mut().expect("never empty");
        match tag {
            Some((end, t)) => {
                last.1.push_str(&rest[..start]);
                segs.push((Some(t), String::new()));
                any = true;
                rest = &rest[start + end + 1..];
            }
            None => {
                last.1.push_str(&rest[..=start]);
                rest = &rest[start + 1..];
            }
        }
    }
    segs.last_mut().expect("never empty").1.push_str(rest);
    let plain: String = segs.iter().map(|s| s.1.as_str()).collect();
    let words = if any {
        segs.into_iter().filter(|s| !s.1.trim().is_empty()).collect()
    } else {
        Vec::new()
    };
    (plain, words)
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

/// Timed lines in play order: sorted by time (stable, so lines sharing a
/// time keep file order), runs of gaps collapsed, no leading or trailing
/// gap, capped.
fn tidy_timed(mut timed: Vec<LyricLine>) -> Vec<LyricLine> {
    timed.sort_by_key(|l| l.start_ms);
    let mut out: Vec<LyricLine> = Vec::new();
    for l in timed {
        if l.text.is_empty() && out.last().is_none_or(|p| p.text.is_empty()) {
            continue;
        }
        out.push(l);
        if out.len() >= MAX_LYRIC_LINES {
            break;
        }
    }
    while out.last().is_some_and(|l| l.text.is_empty()) {
        out.pop();
    }
    out
}

/// A timed line from its start, plain text and word segments.
fn timed_line(start_ms: u32, text: &str, words: &[(Option<u32>, String)]) -> LyricLine {
    let words = words
        .iter()
        .enumerate()
        .map(|(i, (t, w))| LyricWord {
            start_ms: t.unwrap_or(start_ms),
            text: if i == 0 { w.trim_start().to_owned() } else { w.clone() },
        })
        .collect::<Vec<_>>();
    // A trailing space on the last word would only add width.
    let mut words = words;
    if let Some(last) = words.last_mut() {
        last.text = last.text.trim_end().to_owned();
    }
    LyricLine { start_ms, text: text.to_owned(), words }
}

/// Plain or LRC lyrics text. Time tags mark the lyrics as synced and give
/// the timed lines; `<mm:ss.xx>` tags give word timing; `[offset:±ms]` moves
/// every time (positive shows lyrics sooner); other header tags such as
/// `[ar:...]` are left out.
pub fn parse_lyrics_text(text: &str) -> Lyrics {
    let mut lines = Vec::new();
    let mut timed = Vec::new();
    let mut offset: i64 = 0;
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let mut line = raw.trim();
        let mut header = false;
        let mut times = Vec::new();
        while let Some(rest) = line.strip_prefix('[') {
            let Some(end) = rest.find(']') else { break };
            let tag = &rest[..end];
            if let Some(t) = time_ms(tag) {
                times.push(t);
            } else if let Some((k, v)) = tag.split_once(':') {
                header = true;
                if k.trim().eq_ignore_ascii_case("offset") {
                    offset = v.trim().trim_start_matches('+').parse().unwrap_or(0);
                }
            } else {
                break;
            }
            line = rest[end + 1..].trim_start();
        }
        if header && line.is_empty() {
            continue;
        }
        let (plain, words) = split_words(line);
        let plain = plain.trim().to_owned();
        for t in times {
            timed.push(timed_line(t, &plain, &words));
        }
        lines.push(plain);
    }
    if offset != 0 {
        let shift = |t: u32| (t as i64 - offset).clamp(0, u32::MAX as i64) as u32;
        for l in &mut timed {
            l.start_ms = shift(l.start_ms);
            for w in &mut l.words {
                w.start_ms = shift(w.start_ms);
            }
        }
    }
    let timed = tidy_timed(timed);
    Lyrics {
        synced: !timed.is_empty(),
        lines: tidy(lines),
        timed,
        credit: None,
        source: LyricsSource::Plex,
    }
}

/// A JSON number or numeric string as milliseconds.
fn ms_value(v: Option<&Value>) -> Option<u32> {
    let v = v?;
    let n = v.as_u64().or_else(|| v.as_str()?.trim().parse().ok())?;
    u32::try_from(n).ok()
}

/// Lyrics in Plex's JSON form: `Lyrics[].Line[]`, each line a list of
/// `Span` with `text` (or a `text` of its own); timed lines carry
/// `startOffset` in ms, and spans may carry their own (word timing).
fn parse_lyrics_json(v: &Value) -> Option<Lyrics> {
    let lyrics = v.get("MediaContainer")?.get("Lyrics")?.as_array()?;
    let credit = lyrics.first().and_then(lyrics_credit);
    let mut lines = Vec::new();
    let mut timed = Vec::new();
    for l in lyrics.iter().filter_map(|l| l.get("Line")?.as_array()) {
        for line in l {
            let spans = line.get("Span").and_then(Value::as_array);
            let text = match spans {
                Some(spans) => spans.iter().filter_map(|s| s.get("text")?.as_str()).collect::<String>(),
                None => line.get("text").and_then(Value::as_str).unwrap_or_default().to_owned(),
            };
            let text = text.trim().to_owned();
            if let Some(start) = ms_value(line.get("startOffset")) {
                let words: Vec<(Option<u32>, String)> = match spans {
                    Some(spans) if spans.len() > 1 && spans.iter().all(|s| s.get("startOffset").is_some()) => spans
                        .iter()
                        .map(|s| {
                            let t = ms_value(s.get("startOffset"));
                            (t, s.get("text").and_then(Value::as_str).unwrap_or_default().to_owned())
                        })
                        .filter(|(_, w)| !w.trim().is_empty())
                        .collect(),
                    _ => Vec::new(),
                };
                timed.push(timed_line(start, &text, &words));
            }
            lines.push(text);
        }
    }
    let timed = tidy_timed(timed);
    Some(Lyrics {
        synced: !timed.is_empty(),
        lines: tidy(lines),
        timed,
        credit,
        source: LyricsSource::Plex,
    })
}

/// "Written by …" from the author, then the provider's credit line. Plex
/// names LyricFind by its agent id.
fn lyrics_credit(l: &Value) -> Option<String> {
    let text = |k: &str| {
        l.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let mut parts = Vec::new();
    if let Some(a) = text("author").or_else(|| text("by")) {
        parts.push(format!("Written by {a}"));
    }
    if text("provider").is_some_and(|p| p.to_ascii_lowercase().contains("lyricfind")) {
        parts.push("Lyrics licensed and provided by LyricFind".to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(". "))
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
    let key = client.raw(track_id)?;
    if !valid_id(key) {
        return Err(AppError::Invalid("track id".into()));
    }
    let body = client
        .get_bytes(&format!("/library/metadata/{key}"), &[])
        .await?;
    let extras = track_extras(first_meta(&body)?);

    let artist = async {
        let id = extras.artist_id.as_deref().filter(|id| valid_id(id))?;
        let body = client
            .get_bytes(&format!("/library/metadata/{id}"), &[])
            .await
            .map_err(|e| tracing::debug!(kind = e.kind(), "artist details unavailable"))
            .ok()?;
        first_meta(&body).ok().map(artist_about).map(|mut a| {
            // Ids and pictures carry the server's tag like the rest.
            a.id = client.qualify(&a.id);
            a.thumb = a.thumb.map(|t| client.qualify(&t));
            a.art = a.art.map(|t| client.qualify(&t));
            a
        })
    };
    let lyrics = async {
        let path = extras.lyrics_path.as_deref()?;
        let body = client
            .get_bytes(path, &[])
            .await
            .map_err(|e| tracing::debug!(kind = e.kind(), "lyrics unavailable"))
            .ok()?;
        parse_lyrics(&body).map(|l| from_stream(l, extras.lyrics_file))
    };
    let (artist, lyrics) = futures::join!(artist, lyrics);
    Ok(NowPlayingDetails {
        track_id: track_id.to_owned(),
        artist,
        credits: extras.credits,
        lyrics,
    })
}

/// Only the track's lyrics from Plex: None when the track has no lyrics
/// stream or the stream can't be read or has no words. Errors only when the
/// track itself can't be read.
pub async fn fetch_lyrics(client: &PlexClient, track_id: &str) -> AppResult<Option<Lyrics>> {
    let key = client.raw(track_id)?;
    if !valid_id(key) {
        return Err(AppError::Invalid("track id".into()));
    }
    let body = client
        .get_bytes(&format!("/library/metadata/{key}"), &[])
        .await?;
    let extras = track_extras(first_meta(&body)?);
    let Some(path) = extras.lyrics_path else {
        return Ok(None);
    };
    Ok(client
        .get_bytes(&path, &[])
        .await
        .map_err(|e| tracing::debug!(kind = e.kind(), "lyrics unavailable"))
        .ok()
        .and_then(|b| parse_lyrics(&b))
        .map(|l| from_stream(l, extras.lyrics_file)))
}

/// Mark lyrics read from a file next to the song.
fn from_stream(mut l: Lyrics, file: bool) -> Lyrics {
    if file {
        l.source = LyricsSource::File;
    }
    l
}

/// docs/IPC.md `now_playing_details`.
#[tauri::command]
pub async fn now_playing_details(
    state: State<'_, PlexState>,
    backends: State<'_, crate::backend::Backends>,
    track_id: String,
) -> AppResult<NowPlayingDetails> {
    if !backends.caps_for(&track_id).has(crate::backend::Caps::TRACK_DETAILS) {
        return Err(AppError::NotFound("This server has no details for the song.".into()));
    }
    let client = state.client_for(&track_id).await?;
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
        // No agent named: a file next to the song.
        assert!(x.lyrics_file);
    }

    #[test]
    fn lyrics_from_an_agent_are_not_a_file() {
        let mut t = track_fixture();
        t["Media"][0]["Part"][0]["Stream"][2]["provider"] = json!("com.plexapp.agents.lyricfind");
        assert!(!track_extras(meta(t.clone())).lyrics_file);
        t["Media"][0]["Part"][0]["Stream"][2]["provider"] = json!("com.plexapp.agents.localmedia");
        assert!(track_extras(meta(t)).lyrics_file);
        let l = from_stream(parse_lyrics_text("Hello"), true);
        assert_eq!(l.source, LyricsSource::File);
        assert_eq!(from_stream(parse_lyrics_text("Hello"), false).source, LyricsSource::Plex);
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
        // The offset (+100 ms) shows every line sooner; the chorus plays
        // twice; the empty timed line is a gap.
        let times: Vec<(u32, &str)> = l.timed.iter().map(|t| (t.start_ms, t.text.as_str())).collect();
        assert_eq!(
            times,
            vec![(12_200, "First line"), (14_900, "Chorus line"), (19_900, ""), (25_000, "Last line"), (74_900, "Chorus line")]
        );
        assert_eq!(
            l.timed[0].words,
            vec![
                LyricWord { start_ms: 12_200, text: "First ".into() },
                LyricWord { start_ms: 12_700, text: "line".into() }
            ]
        );
        assert!(l.timed[1].words.is_empty());
    }

    #[test]
    fn time_tags_in_every_form() {
        assert_eq!(time_ms("01:23.45"), Some(83_450));
        assert_eq!(time_ms("1:02"), Some(62_000));
        assert_eq!(time_ms("00:01.5"), Some(1_500));
        assert_eq!(time_ms("00:01.234"), Some(1_234));
        assert_eq!(time_ms("00:01:20"), Some(1_200));
        assert_eq!(time_ms("ar:Someone"), None);
        assert_eq!(time_ms("00:1x"), None);
        assert_eq!(time_ms(":12"), None);
    }

    #[test]
    fn bad_lines_and_negative_offsets() {
        let l = parse_lyrics_text("[offset:-500]\n[bad tag\n[00:01.00]One <oops> two\n[99:xx]Not timed\n[00:00.20]Zero");
        // A broken tag is text; an unknown tag is dropped like a header.
        assert_eq!(l.lines, vec!["[bad tag", "One <oops> two", "Not timed", "Zero"]);
        let times: Vec<u32> = l.timed.iter().map(|t| t.start_ms).collect();
        assert_eq!(times, vec![700, 1_500]);
        assert_eq!(l.timed[1].text, "One <oops> two");
    }

    #[test]
    fn word_timing_before_the_first_tag_takes_the_line_start() {
        let l = parse_lyrics_text("[00:10.00]Hold <00:10.50>on <00:11.00>tight <00:12.00>");
        let w: Vec<(u32, &str)> = l.timed[0].words.iter().map(|w| (w.start_ms, w.text.as_str())).collect();
        assert_eq!(w, vec![(10_000, "Hold "), (10_500, "on "), (11_000, "tight")]);
        assert_eq!(l.timed[0].text, "Hold on tight");
    }

    #[test]
    fn structured_timed_spans_give_word_timing() {
        let body = json!({"MediaContainer": {"Lyrics": [{"timed": true, "Line": [
            {"startOffset": 24960, "endOffset": 29100, "Span": [{"text": "one line", "startOffset": 24960, "endOffset": 29100}]},
            {"startOffset": "30000", "Span": [{"text": "two ", "startOffset": 30000}, {"text": "words", "startOffset": 30400}]}
        ]}]}});
        let l = parse_lyrics(&serde_json::to_vec(&body).unwrap()).unwrap();
        assert_eq!(l.timed.len(), 2);
        assert!(l.timed[0].words.is_empty(), "one span is a whole line");
        assert_eq!(l.timed[1].words[1], LyricWord { start_ms: 30_400, text: "words".into() });
        assert_eq!(l.source, LyricsSource::Plex);
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
                lines: vec!["a".into()],
                ..Default::default()
            }),
        };
        let v = serde_json::to_value(d).unwrap();
        assert_eq!(v["trackId"], "1");
        assert!(v["artist"].is_null());
        assert_eq!(v["lyrics"]["lines"][0], "a");
    }

    #[test]
    fn licensed_lyrics_carry_a_credit_line() {
        let body = json!({"MediaContainer": {"size": 1, "Lyrics": [{
            "provider": "com.plexapp.agents.lyricfind", "author": "A. Writer", "timed": "1",
            "Line": [{"startOffset": 0, "Span": [{"text": "first line", "startOffset": 0}]}]
        }]}});
        let l = parse_lyrics(body.to_string().as_bytes()).unwrap();
        assert_eq!(
            l.credit.as_deref(),
            Some("Written by A. Writer. Lyrics licensed and provided by LyricFind")
        );
        let plain = parse_lyrics(b"just words").unwrap();
        assert_eq!(plain.credit, None);
    }
}
