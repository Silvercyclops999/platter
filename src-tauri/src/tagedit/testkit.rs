//! A small library for the tag editor's tests: a music folder of fixture
//! files, the library cache rows Plex would have synced, and a fake Plex.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use parking_lot::Mutex;
use rusqlite::params;

use super::Values;
use super::doc::{Ctx, Doc};
use super::editor::{Options, PlexSide, TagEditor};
use super::fields::Field;
use super::paths::Mapping;
use super::write::{Hooks, NoHooks};
use crate::db::DbHandle;
use crate::error::{AppError, AppResult};

pub const SERVER_ROOT: &str = "/srv/music";

#[derive(Default)]
pub struct FakePlex {
    pub refreshed: Mutex<Vec<(String, String, String)>>,
    pub sizes: Mutex<HashMap<String, u64>>,
    pub down: Mutex<bool>,
}

#[async_trait]
impl PlexSide for FakePlex {
    async fn refresh_folder(&self, track_id: &str, section: &str, folder: &str) -> AppResult<()> {
        if *self.down.lock() {
            return Err(AppError::Network("server unreachable".into()));
        }
        self.refreshed.lock().push((track_id.into(), section.into(), folder.into()));
        Ok(())
    }
    async fn part_sizes(&self, ids: &[String]) -> AppResult<Vec<(String, Option<u64>)>> {
        let s = self.sizes.lock();
        Ok(ids.iter().map(|i| (i.clone(), s.get(i).copied())).collect())
    }
}

pub struct Lib {
    pub music: tempfile::TempDir,
    _db_dir: tempfile::TempDir,
    pub db: DbHandle,
    pub plex: Arc<FakePlex>,
    pub editor: TagEditor,
}

impl Lib {
    pub fn new() -> Self {
        Self::with_hooks(Arc::new(NoHooks))
    }

    /// A library whose writes go through `hooks` (to make them fail).
    pub fn with_hooks(hooks: Arc<dyn Hooks>) -> Self {
        let music = tempfile::tempdir().unwrap();
        let db_dir = tempfile::tempdir().unwrap();
        let db = DbHandle::open(db_dir.path().join("t.db")).unwrap();
        db.write(|c| {
            c.execute(
                "INSERT INTO sync_state (id, section_id, section_uuid) VALUES (1, '7', 'u-7')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let plex = Arc::new(FakePlex::default());
        let editor = TagEditor::new(db.clone(), plex.clone(), hooks, Some(db_dir.path().join("tag-edits.json")));
        let lib = Self { music, _db_dir: db_dir, db, plex, editor };
        lib.editor.set_options(Options { mapping: Some(lib.mapping()), rescan: true });
        lib
    }

    pub fn mapping(&self) -> Mapping {
        Mapping { server: SERVER_ROOT.into(), local: self.music.path().to_string_lossy().to_string() }
    }

    pub fn local(&self, rel: &str) -> PathBuf {
        self.music.path().join(rel)
    }

    /// A song: `fixture` copied to `rel` under the music folder, and its row
    /// as Plex synced it.
    #[allow(clippy::too_many_arguments)]
    pub fn add(&self, id: &str, album: &str, album_title: &str, rel: &str, fixture: &str, disc: i64, track: i64, title: &str) -> PathBuf {
        let dst = self.local(rel);
        std::fs::create_dir_all(dst.parent().unwrap()).unwrap();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tagedit/fixtures").join(fixture);
        std::fs::copy(src, &dst).unwrap();
        age(&dst);
        let server = format!("{SERVER_ROOT}/{rel}");
        self.db
            .write(|c| {
                c.execute(
                    "INSERT OR IGNORE INTO albums (id, title, sort_title, artist_id, artist_title) VALUES (?1, ?2, ?2, 'ar1', 'Demo Artist')",
                    params![album, album_title],
                )?;
                c.execute(
                    "INSERT INTO tracks (id, title, album_id, album_title, artist_id, artist_title, part_key, file,
                                         disc_number, track_number, duration_ms, codec)
                     VALUES (?1, ?2, ?3, ?4, 'ar1', 'Demo Artist', ?5, ?6, ?7, ?8, 1000, ?9)",
                    params![
                        id,
                        title,
                        album,
                        album_title,
                        format!("/library/parts/{id}/1/file"),
                        server,
                        disc,
                        track,
                        dst.extension().unwrap().to_string_lossy().to_string()
                    ],
                )?;
                Ok(())
            })
            .unwrap();
        dst
    }

    /// Changes a file's tags as another program would (and ages it, so
    /// the folder isn't busy).
    pub fn retag(&self, rel: &str, pairs: &[(Field, &str)]) {
        retag(&self.local(rel), pairs, &Ctx::default());
    }
}

pub fn retag(p: &Path, pairs: &[(Field, &str)], ctx: &Ctx) {
    let mut d = Doc::read(p).unwrap();
    for (f, v) in pairs {
        d.set(*f, v, ctx).unwrap();
    }
    d.save(p).unwrap();
    age(p);
}

/// An hour old.
pub fn age(p: &Path) {
    let old = SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options().write(true).open(p).unwrap().set_modified(old).unwrap();
}

pub fn values(p: &Path) -> Values {
    Doc::read(p).unwrap().values(&Ctx { custom_key: crate::tags::provenance::custom_key(), slash_numbers: false })
}
