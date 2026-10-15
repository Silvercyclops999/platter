//! Server-qualified ids.
//!
//! Every Plex item id Platter keeps (cache, IPC, queue, downloads, sync
//! store, links, history) carries the server it came from:
//! `<tag>:<rating key>`, where the tag is 8 hex digits of a hash of the
//! server's machine id (the same tag share links carry). Image paths from a
//! server get the same prefix (`<tag>:/library/metadata/1/thumb/2`), so the
//! art cache knows which server to ask. Part keys stay as Plex sends them;
//! they're always used with their track, whose id names the server.
//!
//! The Plex client adds the tag to everything it reads and takes it off
//! everything it sends, so the rest of the app treats ids as opaque strings.
//!
//! Ids saved before servers were named (a plain rating key, all digits) still
//! work: [`resolve`] maps them to the server the library came from when the
//! cache was rewritten (the "legacy" tag, kept in the cache's `kv` table as
//! `ids.legacyTag`). Links, saved queues, sync store entries written by an
//! older build and ids the UI kept go through it.

#[cfg(not(test))]
use std::sync::RwLock;

/// Length of a server tag in hex digits.
pub const TAG_LEN: usize = 8;

/// The server the library came from before ids were qualified.
#[cfg(not(test))]
static LEGACY: RwLock<Option<String>> = RwLock::new(None);

// Tests run side by side, so each test thread has its own.
#[cfg(test)]
thread_local! {
    static LEGACY: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Short tag for a server's machine id: 8 hex digits of its FNV-1a hash.
pub fn tag(machine_id: &str) -> String {
    let mut h: u32 = 0x811c_9dc5;
    for b in machine_id.as_bytes() {
        h ^= *b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    format!("{h:08x}")
}

/// Whether `s` looks like a server tag.
pub fn is_tag(s: &str) -> bool {
    s.len() == TAG_LEN && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The tag and the server's own id. Ids without a tag come back whole.
pub fn split(id: &str) -> (Option<&str>, &str) {
    match id.split_at_checked(TAG_LEN) {
        Some((t, rest)) if is_tag(t) && rest.starts_with(':') => (Some(t), &rest[1..]),
        _ => (None, id),
    }
}

/// The server tag of an id or image path, if it has one.
pub fn tag_of(id: &str) -> Option<&str> {
    split(id).0
}

/// The id (or path) as its server knows it.
pub fn key(id: &str) -> &str {
    split(id).1
}

/// Whether an id is a plain Plex rating key (digits only): what every Plex
/// item id looked like before tags.
pub fn is_plain_key(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

/// Put `tag` on a rating key or image path from that server. Empty values,
/// values that already have a tag, and things that aren't Plex ids or paths
/// (outside image links like `/ext/...`, local playlist ids) stay as they are.
pub fn qualify(tag: &str, id: &str) -> String {
    if id.is_empty() || split(id).0.is_some() || !(is_plain_key(id) || is_server_path(id)) {
        return id.to_string();
    }
    format!("{tag}:{id}")
}

/// [`qualify`] for an optional value.
pub fn qualify_opt(tag: &str, id: &Option<String>) -> Option<String> {
    id.as_deref().map(|i| qualify(tag, i))
}

/// A path on a Plex server (`/library/...`, `/playlists/...`), as opposed to
/// an outside image link (`/ext/https://...`).
pub fn is_server_path(p: &str) -> bool {
    p.starts_with('/') && !p.starts_with(crate::artistinfo::art::EXT_PREFIX)
}

/// The server the library came from before ids were qualified.
#[cfg(not(test))]
pub fn legacy() -> Option<String> {
    LEGACY.read().ok().and_then(|g| g.clone())
}

/// Set the legacy server tag (after the cache is rewritten, at startup).
#[cfg(not(test))]
pub fn set_legacy(tag: Option<String>) {
    if let Ok(mut g) = LEGACY.write() {
        *g = tag;
    }
}

#[cfg(test)]
pub fn legacy() -> Option<String> {
    LEGACY.with(|l| l.borrow().clone())
}

#[cfg(test)]
pub fn set_legacy(tag: Option<String>) {
    LEGACY.with(|l| *l.borrow_mut() = tag);
}

/// An id from somewhere that may predate tags (a link, a saved queue, the
/// sync file, the UI): plain rating keys get the legacy tag; everything else
/// is returned as is.
pub fn resolve(id: &str) -> String {
    let Some(t) = legacy() else { return id.to_string() };
    if is_plain_key(id) || (is_server_path(id) && tag_of(id).is_none()) {
        return format!("{t}:{id}");
    }
    // Radio seeds name a song or an artist: `track:<id>`, `artist:<id>`.
    for prefix in ["track:", "artist:"] {
        if let Some(rest) = id.strip_prefix(prefix)
            && is_plain_key(rest)
        {
            return format!("{prefix}{t}:{rest}");
        }
    }
    id.to_string()
}

/// Kinds that prefix an id in a key (`album:<id>`, `track:<id>`).
const KINDS: [&str; 4] = ["track:", "album:", "artist:", "playlist:"];

/// A key saved before tags: a plain id, or a kind and a plain id
/// (`album:123`), gets the legacy tag (`album:<tag>:123`).
pub fn resolve_key(key: &str) -> String {
    let Some(t) = legacy() else { return key.to_string() };
    if is_plain_key(key) {
        return format!("{t}:{key}");
    }
    for prefix in KINDS {
        if let Some(rest) = key.strip_prefix(prefix)
            && is_plain_key(rest)
        {
            return format!("{prefix}{t}:{rest}");
        }
    }
    key.to_string()
}

/// The form a key had before tags, when it belongs to the legacy server
/// (`<tag>:123` → `123`, `album:<tag>:123` → `album:123`). None otherwise.
pub fn legacy_variant(key: &str) -> Option<String> {
    let t = legacy()?;
    let (prefix, rest) = KINDS
        .iter()
        .find_map(|p| key.strip_prefix(p).map(|r| (*p, r)))
        .unwrap_or(("", key));
    match split(rest) {
        (Some(tag), k) if tag == t && is_plain_key(k) => Some(format!("{prefix}{k}")),
        _ => None,
    }
}

/// Every id-like string in a JSON value saved before tags (a saved queue):
/// values under id keys and in id lists, resolved with [`resolve`].
pub fn resolve_json(v: &mut serde_json::Value) {
    const ID_KEYS: &[&str] = &["id", "albumId", "artistId", "trackId", "thumb", "target"];
    const ID_LISTS: &[&str] = &["recent", "shuffled", "trackIds", "resume"];
    fn all(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::String(s) => *s = resolve(s),
            serde_json::Value::Array(a) => a.iter_mut().for_each(all),
            serde_json::Value::Object(o) => o.values_mut().for_each(all),
            _ => {}
        }
    }
    match v {
        serde_json::Value::Object(o) => {
            for (k, val) in o.iter_mut() {
                if ID_KEYS.contains(&k.as_str()) {
                    if let serde_json::Value::String(s) = val {
                        *s = resolve(s);
                    }
                } else if ID_LISTS.contains(&k.as_str()) {
                    // Lists of ids, or [context id, resume point] pairs.
                    if let serde_json::Value::Array(a) = val {
                        for item in a {
                            match item {
                                serde_json::Value::Array(pair) => pair.iter_mut().for_each(|p| match p {
                                    serde_json::Value::String(_) => all(p),
                                    other => resolve_json(other),
                                }),
                                serde_json::Value::String(_) => all(item),
                                other => resolve_json(other),
                            }
                        }
                    }
                } else {
                    resolve_json(val);
                }
            }
        }
        serde_json::Value::Array(a) => a.iter_mut().for_each(resolve_json),
        _ => {}
    }
}

/// [`resolve`] for each id.
pub fn resolve_all(ids: &[String]) -> Vec<String> {
    ids.iter().map(|i| resolve(i)).collect()
}

/// The tag an id belongs to, using the legacy tag for plain keys.
pub fn owner(id: &str) -> Option<String> {
    match split(id).0 {
        Some(t) => Some(t.to_string()),
        None if is_plain_key(id) || is_server_path(id) => legacy(),
        None => None,
    }
}

/// The form an id or image path is stored under where it names a file or a
/// cache entry kept from before tags (cached artwork, downloaded covers):
/// the plain key for the legacy server, so those stay found; the id as is
/// for every other server.
pub fn storage_form(id: &str) -> &str {
    match (split(id), legacy()) {
        ((Some(t), k), Some(l)) if t == l => k,
        _ => id,
    }
}

/// An id made safe for a file name: the tag's colon becomes an underscore,
/// so [`from_file_part`] can turn it back.
pub fn file_part(id: &str) -> String {
    match split(id) {
        (Some(t), k) => format!("{t}_{k}"),
        (None, k) => k.to_string(),
    }
}

/// The id a [`file_part`] came from.
pub fn from_file_part(s: &str) -> String {
    match s.split_at_checked(TAG_LEN) {
        Some((t, rest)) if is_tag(t) && rest.starts_with('_') => format!("{t}:{}", &rest[1..]),
        _ => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_the_share_link_server_tag() {
        assert_eq!(tag("machine-a"), crate::share::link::server_tag("machine-a"));
        assert_eq!(tag("machine-a").len(), TAG_LEN);
        assert_ne!(tag("machine-a"), tag("machine-b"));
    }

    #[test]
    fn qualify_and_split_round_trip() {
        let t = tag("m");
        let id = qualify(&t, "123");
        assert_eq!(id, format!("{t}:123"));
        assert_eq!(split(&id), (Some(t.as_str()), "123"));
        assert_eq!(key(&id), "123");
        assert_eq!(qualify(&t, &id), id, "never twice");
        let thumb = qualify(&t, "/library/metadata/1/thumb/2");
        assert_eq!(key(&thumb), "/library/metadata/1/thumb/2");
        assert_eq!(tag_of(&thumb), Some(t.as_str()));
        // Not Plex ids: left alone.
        for other in ["", "local:abc", "daily:1", "/ext/https://plex.tv/users/a/avatar", "track:5"] {
            assert_eq!(qualify(&t, other), other);
            assert_eq!(split(other), (None, other));
        }
        // A mix id that starts with 8 hex letters isn't mistaken for a tag.
        assert_eq!(split("deadbeef-1"), (None, "deadbeef-1"));
    }

    #[test]
    fn plain_ids_resolve_to_the_legacy_server() {
        set_legacy(None);
        assert_eq!(resolve("123"), "123", "nothing to map to yet");
        set_legacy(Some("0a1b2c3d".into()));
        assert_eq!(resolve("123"), "0a1b2c3d:123");
        assert_eq!(resolve("/library/metadata/1/thumb/2"), "0a1b2c3d:/library/metadata/1/thumb/2");
        assert_eq!(resolve("ffffffff:9"), "ffffffff:9");
        assert_eq!(resolve("local:p1"), "local:p1");
        assert_eq!(owner("123").as_deref(), Some("0a1b2c3d"));
        assert_eq!(owner("ffffffff:9").as_deref(), Some("ffffffff"));
        assert_eq!(owner("daily:1"), None);
        set_legacy(None);
    }

    #[test]
    fn a_saved_queue_from_before_tags_resolves() {
        set_legacy(Some("0a1b2c3d".into()));
        let mut v = serde_json::json!({
            "queue": {
                "context": { "kind": "radio", "id": "track:5", "title": "R" },
                "items": [{ "itemId": "7", "track": { "id": "5", "albumId": "4", "artistId": "3", "thumb": "/library/metadata/4/thumb/1", "partKey": "/library/parts/1/2/f.flac", "title": "12" } }],
                "recent": ["5", "6"],
                "resume": [["4", { "index": 0, "trackId": "5", "positionMs": 10 }]],
                "shuffled": ["4", "liked"],
                "nextId": 8
            },
            "positionMs": 3
        });
        resolve_json(&mut v);
        let q = &v["queue"];
        assert_eq!(q["context"]["id"], "track:0a1b2c3d:5");
        assert_eq!(q["items"][0]["itemId"], "7", "the queue's own ids stay");
        assert_eq!(q["items"][0]["track"]["id"], "0a1b2c3d:5");
        assert_eq!(q["items"][0]["track"]["albumId"], "0a1b2c3d:4");
        assert_eq!(q["items"][0]["track"]["thumb"], "0a1b2c3d:/library/metadata/4/thumb/1");
        assert_eq!(q["items"][0]["track"]["partKey"], "/library/parts/1/2/f.flac");
        assert_eq!(q["items"][0]["track"]["title"], "12", "titles are never ids");
        assert_eq!(q["recent"], serde_json::json!(["0a1b2c3d:5", "0a1b2c3d:6"]));
        assert_eq!(q["resume"][0][0], "0a1b2c3d:4");
        assert_eq!(q["resume"][0][1]["trackId"], "0a1b2c3d:5");
        assert_eq!(q["shuffled"], serde_json::json!(["0a1b2c3d:4", "liked"]));
        set_legacy(None);
    }

    #[test]
    fn the_legacy_server_keeps_its_old_storage_names() {
        set_legacy(Some("0a1b2c3d".into()));
        assert_eq!(storage_form("0a1b2c3d:/library/metadata/4/thumb/1"), "/library/metadata/4/thumb/1");
        assert_eq!(storage_form("ffffffff:/library/metadata/4/thumb/1"), "ffffffff:/library/metadata/4/thumb/1");
        assert_eq!(storage_form("/ext/https://x/y.jpg"), "/ext/https://x/y.jpg");
        set_legacy(None);
        assert_eq!(storage_form("0a1b2c3d:9"), "0a1b2c3d:9");
    }

    #[test]
    fn file_names_keep_the_tag() {
        let id = "0a1b2c3d:123";
        assert_eq!(file_part(id), "0a1b2c3d_123");
        assert_eq!(from_file_part(&file_part(id)), id);
        assert_eq!(from_file_part("123"), "123");
        assert!(!file_part(id).contains(':'));
    }
}
