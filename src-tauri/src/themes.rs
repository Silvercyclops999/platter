//! Theme files (docs/IPC.md "Themes"). A theme file is data only: JSON with
//! a name, a base (dark or light) and token values. Only the token names in
//! the UI's contract (src/design/themes/contract.json, shared with the UI's
//! own checks) are taken, and only plain colours or pixel lengths: no CSS,
//! no url(), no scripts. Files live in a `themes` folder in the app's data
//! folder; files that can't be used are listed with the reason.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};

#[derive(Deserialize)]
struct Contract {
    themes: Vec<String>,
    accents: Vec<String>,
    tokens: BTreeMap<String, TokenKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum TokenKind {
    #[serde(rename = "colour")]
    Colour,
    #[serde(rename = "length")]
    Length,
}

static CONTRACT: LazyLock<Contract> =
    LazyLock::new(|| serde_json::from_str(include_str!("../../src/design/themes/contract.json")).expect("theme contract"));

pub const FILE_MAX_BYTES: usize = 64 * 1024;
pub const NAME_MAX: usize = 60;
const VALUE_MAX: usize = 64;
const FIELDS: [&str; 4] = ["name", "base", "tokens", "author"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeBase {
    Dark,
    Light,
}

/// A theme file that passed the checks. The Theme file setting holds one
/// whole, so it travels in the sync file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFile {
    /// The file's name in the themes folder.
    pub file: String,
    pub name: String,
    pub base: ThemeBase,
    pub tokens: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvalidTheme {
    pub file: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFolder {
    pub folder: String,
    pub themes: Vec<ThemeFile>,
    pub invalid: Vec<InvalidTheme>,
}

/// The Theme setting's values, as the UI names them.
pub fn theme_ids() -> &'static [String] {
    &CONTRACT.themes
}

/// Accent colour setting: a preset id or #RRGGBB. Returns the stored form.
pub fn accent_value(v: &str) -> Option<String> {
    let v = v.trim();
    if CONTRACT.accents.iter().any(|a| a == v) {
        return Some(v.to_string());
    }
    let hex = v.strip_prefix('#')?;
    (hex.len() == 6 && hex.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", hex.to_ascii_uppercase()))
}

fn is_number(s: &str) -> bool {
    let (int, frac) = match s.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (s, None),
    };
    !int.is_empty()
        && int.len() <= 3
        && int.chars().all(|c| c.is_ascii_digit())
        && frac.is_none_or(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()))
}

/// A number, a percentage when `pct` is Some(true), either when None.
fn is_part(s: &str, pct: Option<bool>, deg: bool) -> bool {
    let (n, has_pct) = match s.strip_suffix('%') {
        Some(n) => (n, true),
        None => (s, false),
    };
    let n = if deg && !has_pct { n.strip_suffix("deg").unwrap_or(n) } else { n };
    is_number(n) && pct.is_none_or(|p| p == has_pct)
}

pub fn is_colour(v: &str) -> bool {
    if let Some(hex) = v.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.chars().all(|c| c.is_ascii_hexdigit());
    }
    let lower = v.to_ascii_lowercase();
    let Some(open) = lower.find('(') else { return false };
    let func = &lower[..open];
    let Some(inner) = lower[open + 1..].strip_suffix(')') else { return false };
    let rgb = matches!(func, "rgb" | "rgba");
    let hsl = matches!(func, "hsl" | "hsla");
    if !rgb && !hsl {
        return false;
    }
    // The alpha goes after a comma or a slash.
    let parts: Vec<&str> = inner.split([',', ' ', '/', '\t']).filter(|p| !p.is_empty()).collect();
    if !(3..=4).contains(&parts.len()) {
        return false;
    }
    parts.iter().enumerate().all(|(i, p)| match (hsl, i) {
        (false, _) | (true, 3) => is_part(p, None, false),
        (true, 0) => is_part(p, Some(false), true),
        (true, _) => is_part(p, Some(true), false),
    })
}

pub fn is_length(v: &str) -> bool {
    v == "0" || v.strip_suffix("px").is_some_and(is_number)
}

/// Why a token value isn't allowed, or None when it's fine.
fn value_problem(name: &str, kind: TokenKind, value: &Value) -> Option<String> {
    let Value::String(v) = value else { return Some(format!("The value of \"{name}\" isn't text.")) };
    let v = v.trim();
    let lower = v.to_ascii_lowercase();
    let squashed: String = lower.chars().filter(|c| !c.is_whitespace()).collect();
    if squashed.contains("url(") {
        return Some(format!("\"{name}\" uses url(). Theme files can't load anything."));
    }
    let css = v.contains([';', '{', '}', '<', '>', '\\'])
        || lower.contains("/*")
        || squashed.contains("var(")
        || squashed.contains("expression(")
        || lower.contains("@import")
        || lower.contains("javascript:");
    if css {
        return Some(format!("\"{name}\" has CSS in it. Values are plain colours or pixel sizes."));
    }
    if v.chars().count() > VALUE_MAX {
        return Some(format!("The value of \"{name}\" is too long."));
    }
    match kind {
        TokenKind::Colour if !is_colour(v) => Some(format!("\"{name}\" needs a colour like #1A1A1A or rgb(26 26 26).")),
        TokenKind::Length if !is_length(v) => Some(format!("\"{name}\" needs a size in pixels like 8px.")),
        _ => None,
    }
}

/// Check a theme file's parsed JSON.
pub fn validate_value(file: &str, data: &Value) -> Result<ThemeFile, String> {
    let Value::Object(o) = data else {
        return Err("It isn't a theme: expected an object with name, base and tokens.".into());
    };
    if let Some(k) = o.keys().find(|k| !FIELDS.contains(&k.as_str())) {
        return Err(format!("Unknown field \"{k}\". Theme files have name, base and tokens."));
    }
    let name = match o.get("name") {
        Some(Value::String(n)) if !n.trim().is_empty() => n.trim(),
        _ => return Err("It needs a name.".into()),
    };
    if name.chars().count() > NAME_MAX || name.chars().any(|c| c.is_control() || c == '<' || c == '>') {
        return Err(format!("The name can be up to {NAME_MAX} plain characters."));
    }
    if o.get("author").is_some_and(|a| !a.is_string()) {
        return Err("The author isn't text.".into());
    }
    let base = match o.get("base").and_then(Value::as_str) {
        Some("dark") => ThemeBase::Dark,
        Some("light") => ThemeBase::Light,
        _ => return Err("The base must be \"dark\" or \"light\".".into()),
    };
    let Some(Value::Object(tokens)) = o.get("tokens") else {
        return Err("It needs a tokens object.".into());
    };
    if tokens.len() > CONTRACT.tokens.len() {
        return Err("It has more tokens than Platter knows.".into());
    }
    let mut out = BTreeMap::new();
    for (k, v) in tokens {
        let n = k.strip_prefix("--").unwrap_or(k);
        let Some(kind) = CONTRACT.tokens.get(n) else {
            return Err(format!("Unknown token \"{k}\"."));
        };
        if let Some(p) = value_problem(n, *kind, v) {
            return Err(p);
        }
        out.insert(n.to_string(), v.as_str().unwrap_or_default().trim().to_string());
    }
    Ok(ThemeFile { file: file.to_string(), name: name.to_string(), base, tokens: out })
}

/// Check a theme file's text.
pub fn validate(file: &str, text: &str) -> Result<ThemeFile, String> {
    if text.len() > FILE_MAX_BYTES {
        return Err("The file is too big for a theme (64 KB at most).".into());
    }
    let data: Value = serde_json::from_str(text).map_err(|_| "It isn't valid JSON.".to_string())?;
    validate_value(file, &data)
}

/// Check a theme held in settings (it may have come through the sync file).
pub fn check(t: &ThemeFile) -> Result<(), String> {
    let data = serde_json::json!({ "name": t.name, "base": t.base, "tokens": t.tokens });
    validate_value(&t.file, &data).map(|_| ())
}

/// Every .json file in the folder, checked, sorted by file name.
pub fn list_dir(dir: &Path) -> ThemeFolder {
    let mut themes = Vec::new();
    let mut invalid = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")))
                .collect()
        })
        .unwrap_or_default();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    for path in files {
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let read = match std::fs::metadata(&path) {
            Ok(m) if m.len() as usize > FILE_MAX_BYTES => Err("The file is too big for a theme (64 KB at most).".to_string()),
            _ => std::fs::read(&path).map_err(|e| format!("It couldn't be read: {e}")),
        };
        let checked = read.and_then(|b| String::from_utf8(b).map_err(|_| "It isn't text.".to_string())).and_then(|t| validate(&file, &t));
        match checked {
            Ok(t) => themes.push(t),
            Err(reason) => invalid.push(InvalidTheme { file, reason }),
        }
    }
    ThemeFolder { folder: dir.to_string_lossy().into_owned(), themes, invalid }
}

/// A file name in `dir` for a theme called `name` that isn't taken.
fn free_name(dir: &Path, name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' })
        .collect::<String>()
        .trim_matches(['.', ' '])
        .to_string();
    let stem = if stem.is_empty() { "Theme".to_string() } else { stem };
    let mut n = 1;
    loop {
        let file = if n == 1 { format!("{stem}.json") } else { format!("{stem} ({n}).json") };
        if !dir.join(&file).exists() {
            return file;
        }
        n += 1;
    }
}

/// Check a theme file and copy it into the folder. An invalid file isn't copied.
pub fn import_into(dir: &Path, src: &Path) -> AppResult<ThemeFile> {
    let meta = std::fs::metadata(src)?;
    if meta.len() as usize > FILE_MAX_BYTES {
        return Err(AppError::Invalid("The file is too big for a theme (64 KB at most).".into()));
    }
    let text = String::from_utf8(std::fs::read(src)?).map_err(|_| AppError::Invalid("It isn't text.".into()))?;
    let theme = validate("", &text).map_err(AppError::Invalid)?;
    std::fs::create_dir_all(dir)?;
    let file = free_name(dir, &theme.name);
    std::fs::write(dir.join(&file), text)?;
    Ok(ThemeFile { file, ..theme })
}

fn themes_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app.path().app_data_dir().map_err(|e| AppError::Internal(format!("data folder: {e}")))?.join("themes");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[tauri::command]
pub async fn themes_list(app: AppHandle) -> AppResult<ThemeFolder> {
    let dir = themes_dir(&app)?;
    tokio::task::spawn_blocking(move || list_dir(&dir)).await.map_err(|e| AppError::Internal(format!("themes: {e}")))
}

/// Pick a theme file and copy it into the themes folder. None when cancelled.
#[tauri::command]
pub async fn themes_import(app: AppHandle) -> AppResult<Option<ThemeFile>> {
    let dir = themes_dir(&app)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title("Import a theme").add_filter("Theme file", &["json"]).pick_file(move |p| {
        let _ = tx.send(p);
    });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let src = picked.into_path().map_err(|e| AppError::Io(format!("theme path: {e}")))?;
    tokio::task::spawn_blocking(move || import_into(&dir, &src))
        .await
        .map_err(|e| AppError::Internal(format!("import: {e}")))?
        .map(Some)
}

/// Show the themes folder in Explorer / Finder.
#[tauri::command]
pub async fn themes_open_folder(app: AppHandle) -> AppResult<()> {
    let dir = themes_dir(&app)?;
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| AppError::Internal(format!("open folder: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Case {
        case: String,
        text: String,
        ok: bool,
        reason: Option<String>,
    }

    #[test]
    fn shared_validation_cases() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("../../src/design/themes/validation-cases.json")).unwrap();
        assert!(cases.len() >= 10);
        for c in cases {
            match (validate("t.json", &c.text), c.ok) {
                (Ok(_), true) => {}
                (Err(r), false) => {
                    let want = c.reason.unwrap_or_default();
                    assert!(r.contains(&want), "{}: got {r:?}, want {want:?}", c.case);
                }
                (got, _) => panic!("{}: unexpected {got:?}", c.case),
            }
        }
    }

    #[test]
    fn keeps_values_and_strips_dashes() {
        let t = validate("a.json", r##"{"name":" Sea ","base":"light","tokens":{"--color-accent":" #112233 ","radius-card":"10px"}}"##).unwrap();
        assert_eq!(t.name, "Sea");
        assert_eq!(t.base, ThemeBase::Light);
        assert_eq!(t.tokens.get("color-accent").map(String::as_str), Some("#112233"));
        assert_eq!(t.tokens.get("radius-card").map(String::as_str), Some("10px"));
        assert!(check(&t).is_ok());
    }

    #[test]
    fn colours_and_lengths() {
        for ok in ["#fff", "#FFFF", "#0e0d0c", "#0E0D0C80", "rgb(1 2 3)", "rgba(255, 240, 220, 0.06)", "rgb(10% 20% 30% / 0.5)", "hsl(120deg 50% 40%)", "hsla(20, 30%, 40%, 0.5)"] {
            assert!(is_colour(ok), "{ok}");
        }
        for bad in ["red", "#ggg", "#12345", "rgb(1 2)", "rgb(1 2 3 4 5)", "hsl(1 2 3)", "rgb(1000 1 1)", "calc(1px)", "rgb(1 2 3"] {
            assert!(!is_colour(bad), "{bad}");
        }
        assert!(is_length("0") && is_length("8px") && is_length("999px") && is_length("1.5px"));
        assert!(!is_length("8") && !is_length("2em") && !is_length("1000px") && !is_length("-1px"));
    }

    #[test]
    fn accent_values() {
        assert_eq!(accent_value("ivory").as_deref(), Some("ivory"));
        assert_eq!(accent_value("teal").as_deref(), Some("teal"));
        assert_eq!(accent_value("#a1b2c3").as_deref(), Some("#A1B2C3"));
        assert_eq!(accent_value("purple"), None);
        assert_eq!(accent_value("#abc"), None);
        assert_eq!(accent_value("#12345g"), None);
    }

    #[test]
    fn lists_good_and_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.json"), r##"{"name":"B","base":"dark","tokens":{"color-bg-base":"#000000"}}"##).unwrap();
        std::fs::write(dir.path().join("a.json"), r#"{"name":"A","base":"dark","tokens":{"color-bg-base":"url(x)"}}"#).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a theme").unwrap();
        std::fs::write(dir.path().join("big.json"), " ".repeat(FILE_MAX_BYTES + 1)).unwrap();
        let f = list_dir(dir.path());
        assert_eq!(f.themes.iter().map(|t| t.file.as_str()).collect::<Vec<_>>(), ["b.json"]);
        assert_eq!(f.invalid.iter().map(|t| t.file.as_str()).collect::<Vec<_>>(), ["a.json", "big.json"]);
        assert!(f.invalid[0].reason.contains("url()"));
        assert!(f.invalid[1].reason.contains("too big"));
    }

    #[test]
    fn missing_folder_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let f = list_dir(&dir.path().join("nope"));
        assert!(f.themes.is_empty() && f.invalid.is_empty());
    }

    #[test]
    fn import_copies_valid_files_under_a_free_name() {
        let src = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let good = src.path().join("x.json");
        std::fs::write(&good, r#"{"name":"Sea / Sky","base":"light","tokens":{}}"#).unwrap();
        let a = import_into(dest.path(), &good).unwrap();
        let b = import_into(dest.path(), &good).unwrap();
        assert_eq!(a.file, "Sea _ Sky.json");
        assert_eq!(b.file, "Sea _ Sky (2).json");
        assert_eq!(list_dir(dest.path()).themes.len(), 2);

        let bad = src.path().join("bad.json");
        std::fs::write(&bad, r#"{"name":"Bad","base":"dark","tokens":{"color-accent":"red; }"}}"#).unwrap();
        let err = import_into(dest.path(), &bad).unwrap_err();
        assert!(matches!(err, AppError::Invalid(ref m) if m.contains("CSS")), "{err:?}");
        assert_eq!(list_dir(dest.path()).themes.len(), 2);
    }

    #[test]
    fn contract_lists_the_theme_setting_values() {
        let ids: Vec<String> = theme_ids().to_vec();
        for mode in [
            crate::settings::ThemeMode::Dark,
            crate::settings::ThemeMode::Light,
            crate::settings::ThemeMode::System,
            crate::settings::ThemeMode::Midnight,
            crate::settings::ThemeMode::Forest,
            crate::settings::ThemeMode::Sand,
        ] {
            let name = serde_json::to_value(mode).unwrap();
            assert!(ids.iter().any(|i| Some(i.as_str()) == name.as_str()), "{name}");
        }
        assert_eq!(ids.len(), 6);
    }
}
