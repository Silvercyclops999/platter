//! Reading what the user picked: one file, a folder of files, or a zip (as
//! account data downloads come), and zips inside a picked folder.
//!
//! Only the kinds of file an import can use are read (JSON, CSV, M3U and
//! plain text lists). Sizes are capped so a damaged or hostile zip can't
//! fill memory.

use std::io::Read;
use std::path::Path;

use crate::error::{AppError, AppResult};

/// Largest single file read.
pub const MAX_FILE_BYTES: u64 = 512 * 1024 * 1024;
/// Largest total read from one pick.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Deepest folder looked into.
const MAX_DEPTH: usize = 6;
/// Entries a zip may hold before Platter refuses it (account data downloads
/// have a few dozen).
const MAX_ZIP_ENTRIES: usize = 10_000;

/// One file from the pick: its name (the path inside the zip or folder) and
/// its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl InputFile {
    pub fn new(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self { name: name.into(), bytes: bytes.into() }
    }

    /// Lower-case extension, without the dot.
    pub fn ext(&self) -> String {
        ext_of(&self.name)
    }

    /// The file name without folders or extension.
    pub fn stem(&self) -> String {
        let base = self.name.rsplit(['/', '\\']).next().unwrap_or(&self.name);
        match base.rfind('.') {
            Some(i) if i > 0 => base[..i].to_string(),
            _ => base.to_string(),
        }
    }
}

fn ext_of(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.rfind('.').map(|i| base[i + 1..].to_ascii_lowercase()).unwrap_or_default()
}

/// Kinds of file an import reads.
pub fn wanted(name: &str) -> bool {
    matches!(ext_of(name).as_str(), "json" | "csv" | "tsv" | "txt" | "m3u" | "m3u8")
}

struct Budget {
    left: u64,
}

impl Budget {
    fn take(&mut self, n: u64) -> AppResult<()> {
        if n > MAX_FILE_BYTES || n > self.left {
            return Err(AppError::Invalid("That's more than Platter can import at once. Pick fewer files.".into()));
        }
        self.left -= n;
        Ok(())
    }
}

/// Reads every usable file at `path` (a file, a folder or a zip).
pub fn read_input(path: &Path) -> AppResult<Vec<InputFile>> {
    let mut out = Vec::new();
    let mut budget = Budget { left: MAX_TOTAL_BYTES };
    if path.is_dir() {
        read_dir(path, path, 0, &mut budget, &mut out)?;
    } else if ext_of(&path.to_string_lossy()) == "zip" {
        let f = std::fs::File::open(path)?;
        read_zip(f, "", &mut budget, &mut out)?;
    } else {
        let len = std::fs::metadata(path)?.len();
        budget.take(len)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        out.push(InputFile::new(name, std::fs::read(path)?));
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

fn read_dir(root: &Path, dir: &Path, depth: usize, budget: &mut Budget, out: &mut Vec<InputFile>) -> AppResult<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let p = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            read_dir(root, &p, depth + 1, budget, out)?;
        } else if ft.is_file() {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            if ext_of(&rel) == "zip" {
                read_zip(std::fs::File::open(&p)?, &format!("{rel}/"), budget, out)?;
            } else if wanted(&rel) {
                budget.take(entry.metadata()?.len())?;
                out.push(InputFile::new(rel, std::fs::read(&p)?));
            }
        }
    }
    Ok(())
}

/// Reads the usable files inside a zip. `prefix` goes before each name.
fn read_zip<R: Read + std::io::Seek>(r: R, prefix: &str, budget: &mut Budget, out: &mut Vec<InputFile>) -> AppResult<()> {
    let mut z = zip::ZipArchive::new(r).map_err(|e| AppError::Decode(format!("That zip couldn't be opened ({e}).")))?;
    if z.len() > MAX_ZIP_ENTRIES {
        return Err(AppError::Invalid("That zip has too many files to import.".into()));
    }
    for i in 0..z.len() {
        let mut f = z.by_index(i).map_err(|e| AppError::Decode(format!("zip entry: {e}")))?;
        // A name that isn't valid text is nothing an import reads.
        let Ok(entry) = f.name().map(|n| n.into_owned()) else { continue };
        if f.is_dir() || !wanted(&entry) {
            continue;
        }
        let name = format!("{prefix}{entry}");
        // The size in the header can lie (a zip bomb says 1 byte and holds
        // gigabytes): read at most what's left of the budget, then charge
        // what was really read.
        let cap = budget.left.min(MAX_FILE_BYTES);
        let mut bytes = Vec::with_capacity(f.size().min(cap) as usize);
        (&mut f).take(cap + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(AppError::Invalid("A file in that zip is too large to import.".into()));
        }
        budget.take(bytes.len() as u64)?;
        out.push(InputFile::new(name, bytes));
    }
    Ok(())
}

/// Reads a zip held in memory.
pub fn read_zip_bytes(bytes: &[u8]) -> AppResult<Vec<InputFile>> {
    let mut out = Vec::new();
    let mut budget = Budget { left: MAX_TOTAL_BYTES };
    read_zip(std::io::Cursor::new(bytes), "", &mut budget, &mut out)?;
    Ok(out)
}

/// Text from a file's bytes: UTF-8 (with or without a byte order mark),
/// UTF-16 with a byte order mark, or Latin-1 (older spreadsheet exports).
/// A UTF-8 file with a stray bad byte stays UTF-8 (that byte becomes a
/// replacement mark); only a file with no UTF-8 in it at all is Latin-1.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], le: bool| {
        let units: Vec<u16> =
            rest.as_chunks::<2>().0.iter().map(|c| if le { u16::from_le_bytes(*c) } else { u16::from_be_bytes(*c) }).collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, true);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, false);
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) if has_utf8_sequences(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

/// Whether the bytes hold any well-formed multi-byte UTF-8 character.
fn has_utf8_sequences(bytes: &[u8]) -> bool {
    bytes.utf8_chunks().any(|c| !c.valid().is_ascii())
}
