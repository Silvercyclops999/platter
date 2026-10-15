//! Each album cover's size in pixels, read from the first bytes of the
//! image Plex keeps (never the resized copies the app shows, which Plex
//! scales up). Reads go through the tag scan's reader: lowest priority on
//! the player's transfer gate, waiting while playback downloads.

use std::io::Cursor;
use std::time::Duration;

use rusqlite::{Connection, params};

use crate::db::DbHandle;
use crate::error::AppResult;
use crate::tags::fetch::{FetchError, Reader};
use crate::tags::scan::PartSource;

/// The first read, and the bigger one when the image's size comes later
/// (a large metadata block ahead of it).
const FIRST_READ: usize = 64 * 1024;
const SECOND_READ: usize = 512 * 1024;

/// Album covers not sized yet, or whose cover changed, newest albums first.
pub fn pending(c: &Connection, limit: usize) -> AppResult<Vec<(String, String)>> {
    let mut st = c.prepare_cached(
        "SELECT al.id, al.thumb FROM albums al
         LEFT JOIN doctor_covers dc ON dc.album_id = al.id
         WHERE al.thumb IS NOT NULL AND al.thumb <> '' AND (dc.album_id IS NULL OR dc.thumb <> al.thumb)
         ORDER BY al.added_at DESC, al.id LIMIT ?1",
    )?;
    let out = st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<Vec<_>, _>>()?;
    Ok(out)
}

/// Keep a cover's size; None when it couldn't be read (not asked again
/// until the cover changes).
pub fn save(c: &Connection, album_id: &str, thumb: &str, dims: Option<(u32, u32)>, now: i64) -> AppResult<()> {
    c.execute(
        "INSERT OR REPLACE INTO doctor_covers (album_id, thumb, width, height, checked_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![album_id, thumb, dims.map(|d| d.0), dims.map(|d| d.1), now],
    )?;
    Ok(())
}

/// Width and height from an image's first bytes (JPEG or PNG).
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
        .filter(|(w, h)| *w > 0 && *h > 0)
}

/// What sizing one cover came to.
enum Sized {
    Got(Option<(u32, u32)>),
    /// Network or server trouble: stop for now, try again next round.
    Later,
}

async fn size_one(parts: &dyn PartSource, reader: &Reader, album_id: &str, thumb: &str) -> Sized {
    if !crate::sid::split(thumb).1.starts_with('/') {
        // A full address outside the server (an agent's image): nothing to read.
        return Sized::Got(None);
    }
    let src = match parts.part(album_id, thumb).await {
        Ok(s) => s,
        Err(_) => return Sized::Later,
    };
    for len in [FIRST_READ, SECOND_READ] {
        match reader.range(&src, 0, len).await {
            Ok(got) => {
                if let Some(d) = dimensions(&got.bytes) {
                    return Sized::Got(Some(d));
                }
                if got.whole || got.bytes.len() < len {
                    return Sized::Got(None);
                }
            }
            Err(FetchError::Transient(_)) => return Sized::Later,
            Err(_) => return Sized::Got(None),
        }
    }
    Sized::Got(None)
}

/// Size up to `limit` covers, `gap` apart. Returns how many were sized.
pub async fn probe_round(db: &DbHandle, parts: &dyn PartSource, reader: &Reader, limit: usize, gap: Duration) -> AppResult<usize> {
    let todo = db.read_async(move |c| pending(c, limit)).await?;
    let mut n = 0;
    for (album, thumb) in todo {
        if !super::options().on || !super::options().checks.covers {
            break;
        }
        match size_one(parts, reader, &album, &thumb).await {
            Sized::Got(dims) => {
                let now = super::now_s();
                db.write_async(move |c| save(c, &album, &thumb, dims, now)).await?;
                n += 1;
            }
            Sized::Later => break,
        }
        tokio::time::sleep(gap).await;
    }
    Ok(n)
}
