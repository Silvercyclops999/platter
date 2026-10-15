//! Findings in SQLite: what each album artist's last check found, plus the
//! cover findings, which are worked out when asked (the low-res size is a
//! setting, so changing it needs no new check).

use rusqlite::{Connection, OptionalExtension, params};

use super::{Finding, Fix, Kind, Options};
use crate::error::AppResult;

/// What changes in the library that the checks depend on. When it moves,
/// a check is due.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    pub synced: Option<i64>,
    pub albums: i64,
    pub tracks: i64,
    pub tags: i64,
    pub tags_at: i64,
    pub mb_at: i64,
}

pub fn stamp(c: &Connection) -> AppResult<Stamp> {
    Ok(c.query_row(
        "SELECT (SELECT max(last_sync_at) FROM sync_state),
                (SELECT count(*) FROM albums),
                (SELECT count(*) FROM tracks),
                (SELECT count(*) FROM file_tags),
                (SELECT COALESCE(max(scanned_at), 0) FROM file_tags),
                (SELECT COALESCE(max(checked_at), 0) FROM album_mb_check)",
        [],
        |r| Ok(Stamp { synced: r.get(0)?, albums: r.get(1)?, tracks: r.get(2)?, tags: r.get(3)?, tags_at: r.get(4)?, mb_at: r.get(5)? }),
    )?)
}

/// Every album artist, in a steady order.
pub fn artist_ids(c: &Connection) -> AppResult<Vec<String>> {
    let mut st = c.prepare_cached("SELECT DISTINCT artist_id FROM albums ORDER BY artist_id")?;
    let out = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Keep one artist's new findings and what the check saw.
pub fn replace_artist(c: &mut Connection, artist_id: &str, sig: &str, findings: &[Finding]) -> AppResult<()> {
    let tx = c.transaction()?;
    tx.execute("DELETE FROM doctor_findings WHERE artist_id = ?1", [artist_id])?;
    {
        let mut ins = tx.prepare_cached("INSERT OR REPLACE INTO doctor_findings (id, artist_id, kind, body) VALUES (?1, ?2, ?3, ?4)")?;
        for f in findings {
            ins.execute(params![f.id, artist_id, f.kind.key(), serde_json::to_string(f).unwrap_or_default()])?;
        }
    }
    tx.execute(
        "INSERT OR REPLACE INTO doctor_artists (artist_id, sig, checked_at) VALUES (?1, ?2, ?3)",
        params![artist_id, sig, super::now_s()],
    )?;
    tx.commit()?;
    Ok(())
}

/// Drop what's kept for artists and albums no longer in the library.
pub fn prune(c: &mut Connection) -> AppResult<usize> {
    let mut n = c.execute("DELETE FROM doctor_findings WHERE artist_id NOT IN (SELECT artist_id FROM albums)", [])?;
    n += c.execute("DELETE FROM doctor_artists WHERE artist_id NOT IN (SELECT artist_id FROM albums)", [])?;
    n += c.execute("DELETE FROM doctor_covers WHERE album_id NOT IN (SELECT id FROM albums)", [])?;
    Ok(n)
}

/// Forget what each check saw, so the next round checks every artist.
pub fn forget(c: &mut Connection) -> AppResult<()> {
    c.execute("DELETE FROM doctor_artists", [])?;
    Ok(())
}

/// Stored findings (every kind but covers).
pub fn stored(c: &Connection) -> AppResult<Vec<Finding>> {
    let mut st = c.prepare_cached("SELECT body FROM doctor_findings")?;
    let rows = st.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for b in rows {
        if let Ok(f) = serde_json::from_str::<Finding>(&b?) {
            out.push(f);
        }
    }
    Ok(out)
}

/// Albums with no cover, or a cover smaller than `px` on its longest side.
pub fn cover_findings(c: &Connection, px: u32) -> AppResult<Vec<Finding>> {
    let mut st = c.prepare_cached(
        "SELECT al.id, al.title, al.artist_id, al.artist_title, al.thumb, dc.width, dc.height
         FROM albums al
         LEFT JOIN doctor_covers dc ON dc.album_id = al.id AND dc.thumb = al.thumb
         WHERE al.thumb IS NULL OR al.thumb = ''
            OR (dc.width IS NOT NULL AND dc.height IS NOT NULL AND max(dc.width, dc.height) < ?1)
         ORDER BY al.artist_title COLLATE NOCASE, al.title COLLATE NOCASE",
    )?;
    let rows = st.query_map([px], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, Option<u32>>(5)?,
            r.get::<_, Option<u32>>(6)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, title, artist_id, artist, thumb, w, h) = row?;
        let thumb = thumb.filter(|t| !t.is_empty());
        let (headline, text, evidence) = match (thumb.is_some(), w, h) {
            (true, Some(w), Some(h)) => (
                format!("Small cover ({w} \u{d7} {h})"),
                format!("The cover is smaller than {px} pixels, so it looks soft on big screens and in full window. Change cover finds a bigger one."),
                vec![format!("The cover is {w} by {h} pixels; covers under {px} pixels on their longest side count as small (Settings).")],
            ),
            _ => (
                "No cover".to_string(),
                "This album has no cover. Change cover lets you pick one from Plex's choices or use your own image.".to_string(),
                vec!["This album has no cover.".to_string()],
            ),
        };
        out.push(Finding {
            id: format!("cover:{id}"),
            kind: Kind::Cover,
            artist_id,
            artist,
            album_id: id,
            album: title,
            thumb,
            title: headline,
            text,
            evidence,
            fix: Fix::Cover,
            tracks: Vec::new(),
            others: Vec::new(),
            table: Vec::new(),
            suggest: None,
        });
    }
    Ok(out)
}

/// Every finding for the checks that are on, in page order: by kind, then
/// artist and album.
pub fn all(c: &Connection, o: &Options) -> AppResult<Vec<Finding>> {
    let mut out: Vec<Finding> = stored(c)?.into_iter().filter(|f| o.checks.on(f.kind)).collect();
    if o.checks.covers {
        out.extend(cover_findings(c, o.cover_px)?);
    }
    if o.checks.sounds_like {
        out.extend(super::sounds::findings(c)?);
    }
    out.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.artist.to_lowercase().cmp(&b.artist.to_lowercase()))
            .then_with(|| a.album.to_lowercase().cmp(&b.album.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    Ok(out)
}

/// When an artist was last checked (tests and diagnostics).
pub fn checked(c: &Connection, artist_id: &str) -> AppResult<Option<i64>> {
    Ok(c.query_row("SELECT checked_at FROM doctor_artists WHERE artist_id = ?1", [artist_id], |r| r.get(0)).optional()?)
}
