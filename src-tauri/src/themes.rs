//! Theme and look files (docs/IPC.md "Themes and looks"). Both are data only.
//! A theme file is JSON with a name, a base (dark or light) and token values.
//! A look file (`.platter-look`) adds a format version, an author, a style
//! (density, shadow, blur, border), bundled fonts and a layout. Only the names
//! in the UI's contract (src/design/themes/contract.json, shared with the UI's
//! own checks) are taken, and only plain colours, pixel lengths and the listed
//! choices: no CSS, no url(), no scripts. Files live in a `themes` folder in
//! the app's data folder; files that can't be used are listed with the reason.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};

#[derive(Deserialize)]
struct Contract {
    themes: Vec<String>,
    accents: Vec<String>,
    tokens: BTreeMap<String, TokenKind>,
    look: LookContract,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LookContract {
    version: u64,
    fonts: Vec<String>,
    density: Vec<String>,
    shadow: Vec<String>,
    blur: Vec<String>,
    border_max: u8,
    layout: LayoutContract,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LayoutContract {
    sidebar_side: Vec<String>,
    now_playing: Vec<String>,
    player_style: Vec<String>,
    header_style: Vec<String>,
    cover_shape: Vec<String>,
    motion: Vec<String>,
    quick_access: Vec<u8>,
    sidebar_sections: Vec<String>,
    player_buttons: Vec<String>,
    home_shelves: Vec<String>,
    track_columns: Vec<String>,
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
const LOOK_FIELDS: [&str; 8] = ["version", "name", "author", "base", "tokens", "style", "fonts", "layout"];
/// The file extension of a look.
pub const LOOK_EXT: &str = "platter-look";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThemeBase {
    Dark,
    Light,
}

/// Surfaces: density, shadow depth, blur and border weight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LookStyle {
    pub density: String,
    pub shadow: String,
    pub blur: String,
    /// Border weight in pixels, 0 up to the contract's borderMax.
    pub border: u8,
}

impl Default for LookStyle {
    fn default() -> Self {
        Self { density: "comfortable".into(), shadow: "flat".into(), blur: "off".into(), border: 0 }
    }
}

/// Bundled fonts for text and for headings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LookFonts {
    pub text: String,
    pub headings: String,
}

impl Default for LookFonts {
    fn default() -> Self {
        Self { text: "inter".into(), headings: "inter".into() }
    }
}

/// The layout options. The default is Classic, the layout Platter has always
/// had. Lists hold the shown items in order; an item left out is hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Layout {
    pub sidebar_side: String,
    pub sidebar_collapsed: bool,
    pub sidebar_sections: Vec<String>,
    pub now_playing: String,
    pub player_style: String,
    pub player_buttons: Vec<String>,
    pub home_shelves: Vec<String>,
    pub home_greeting: bool,
    pub quick_access: u8,
    pub header_style: String,
    pub track_columns: Vec<String>,
    pub cover_shape: String,
    pub spin_record: bool,
    pub motion: String,
}

impl Default for Layout {
    fn default() -> Self {
        let c = &CONTRACT.look.layout;
        Self {
            sidebar_side: "left".into(),
            sidebar_collapsed: false,
            sidebar_sections: c.sidebar_sections.clone(),
            now_playing: "right".into(),
            player_style: "classic".into(),
            player_buttons: c.player_buttons.clone(),
            home_shelves: c.home_shelves.clone(),
            home_greeting: false,
            quick_access: 8,
            header_style: "bigArt".into(),
            track_columns: c.track_columns.clone(),
            cover_shape: "square".into(),
            spin_record: false,
            motion: "full".into(),
        }
    }
}

/// A theme or look file that passed the checks. The Theme file setting holds
/// one whole, so it travels in the sync file. Older theme files fill only
/// the first four fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFile {
    /// The file's name in the themes folder.
    pub file: String,
    pub name: String,
    pub base: ThemeBase,
    pub tokens: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<LookStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fonts: Option<LookFonts>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// Things in the file that were left out (names Platter doesn't know).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl ThemeFile {
    fn is_look(&self) -> bool {
        self.file.to_ascii_lowercase().ends_with(LOOK_EXT)
            || self.author.is_some()
            || self.style.is_some()
            || self.fonts.is_some()
            || self.layout.is_some()
    }
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

/// A name or author: plain text, up to NAME_MAX characters.
fn plain_text(v: &str) -> bool {
    v.chars().count() <= NAME_MAX && !v.chars().any(|c| c.is_control() || c == '<' || c == '>')
}

/// A key from a file, made safe and short enough to show in a note.
fn shown_key(k: &str) -> String {
    k.chars().filter(|c| !c.is_control()).take(40).collect()
}

/// Name, author and base, shared by theme and look files.
fn header(o: &Map<String, Value>) -> Result<(String, Option<String>, ThemeBase), String> {
    let name = match o.get("name") {
        Some(Value::String(n)) if !n.trim().is_empty() => n.trim(),
        _ => return Err("It needs a name.".into()),
    };
    if !plain_text(name) {
        return Err(format!("The name can be up to {NAME_MAX} plain characters."));
    }
    let author = match o.get("author") {
        None | Some(Value::Null) => None,
        Some(Value::String(a)) if plain_text(a.trim()) => Some(a.trim().to_string()).filter(|a| !a.is_empty()),
        Some(Value::String(_)) => return Err(format!("The author can be up to {NAME_MAX} plain characters.")),
        Some(_) => return Err("The author isn't text.".into()),
    };
    let base = match o.get("base").and_then(Value::as_str) {
        Some("dark") => ThemeBase::Dark,
        Some("light") => ThemeBase::Light,
        _ => return Err("The base must be \"dark\" or \"light\".".into()),
    };
    Ok((name.to_string(), author, base))
}

/// Check a theme or look file's parsed JSON. A file with a `version` is a
/// look; one without is an older theme file, checked as before.
pub fn validate_value(file: &str, data: &Value) -> Result<ThemeFile, String> {
    let Value::Object(o) = data else {
        return Err("It isn't a theme: expected an object with name, base and tokens.".into());
    };
    if o.contains_key("version") {
        return validate_look(file, o);
    }
    if let Some(k) = o.keys().find(|k| !FIELDS.contains(&k.as_str())) {
        return Err(format!("Unknown field \"{k}\". Theme files have name, base and tokens."));
    }
    let (name, author, base) = header(o)?;
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
    Ok(ThemeFile { file: file.to_string(), name, base, tokens: out, author, style: None, fonts: None, layout: None, notes: Vec::new() })
}

/// One of the listed choices.
fn pick(field: &str, v: &Value, allowed: &[String]) -> Result<String, String> {
    match v.as_str() {
        Some(s) if allowed.iter().any(|a| a == s) => Ok(s.to_string()),
        _ => Err(format!("\"{field}\" must be one of: {}.", allowed.join(", "))),
    }
}

fn flag(field: &str, v: &Value) -> Result<bool, String> {
    v.as_bool().ok_or_else(|| format!("\"{field}\" must be true or false."))
}

/// A list of known ids, each once.
fn id_list(field: &str, v: &Value, allowed: &[String]) -> Result<Vec<String>, String> {
    let Value::Array(items) = v else { return Err(format!("\"{field}\" must be a list.")) };
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let id = pick(field, item, allowed)?;
        if out.contains(&id) {
            return Err(format!("\"{field}\" lists \"{id}\" twice."));
        }
        out.push(id);
    }
    Ok(out)
}

fn object<'a>(field: &str, v: &'a Value) -> Result<&'a Map<String, Value>, String> {
    v.as_object().ok_or_else(|| format!("\"{field}\" must be an object."))
}

fn parse_style(v: &Value, notes: &mut Vec<String>) -> Result<LookStyle, String> {
    let c = &CONTRACT.look;
    let mut s = LookStyle::default();
    for (k, val) in object("style", v)? {
        match k.as_str() {
            "density" => s.density = pick("style.density", val, &c.density)?,
            "shadow" => s.shadow = pick("style.shadow", val, &c.shadow)?,
            "blur" => s.blur = pick("style.blur", val, &c.blur)?,
            "border" => {
                s.border = val
                    .as_u64()
                    .filter(|n| *n <= u64::from(c.border_max))
                    .and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| format!("\"style.border\" must be a whole number from 0 to {}.", c.border_max))?
            }
            _ => notes.push(format!("Left out style.{}: Platter doesn't know it.", shown_key(k))),
        }
    }
    Ok(s)
}

fn parse_fonts(v: &Value, notes: &mut Vec<String>) -> Result<LookFonts, String> {
    let fonts = &CONTRACT.look.fonts;
    let mut f = LookFonts::default();
    for (k, val) in object("fonts", v)? {
        match k.as_str() {
            "text" => f.text = pick("fonts.text", val, fonts)?,
            "headings" => f.headings = pick("fonts.headings", val, fonts)?,
            _ => notes.push(format!("Left out fonts.{}: Platter doesn't know it.", shown_key(k))),
        }
    }
    Ok(f)
}

fn parse_layout(v: &Value, notes: &mut Vec<String>) -> Result<Layout, String> {
    let c = &CONTRACT.look.layout;
    let mut l = Layout::default();
    for (k, val) in object("layout", v)? {
        let f = format!("layout.{}", shown_key(k));
        match k.as_str() {
            "sidebarSide" => l.sidebar_side = pick(&f, val, &c.sidebar_side)?,
            "sidebarCollapsed" => l.sidebar_collapsed = flag(&f, val)?,
            "sidebarSections" => l.sidebar_sections = id_list(&f, val, &c.sidebar_sections)?,
            "nowPlaying" => l.now_playing = pick(&f, val, &c.now_playing)?,
            "playerStyle" => l.player_style = pick(&f, val, &c.player_style)?,
            "playerButtons" => l.player_buttons = id_list(&f, val, &c.player_buttons)?,
            "homeShelves" => l.home_shelves = id_list(&f, val, &c.home_shelves)?,
            "homeGreeting" => l.home_greeting = flag(&f, val)?,
            "quickAccess" => {
                l.quick_access = val
                    .as_u64()
                    .and_then(|n| u8::try_from(n).ok())
                    .filter(|n| c.quick_access.contains(n))
                    .ok_or_else(|| format!("\"{f}\" must be 4, 6 or 8."))?
            }
            "headerStyle" => l.header_style = pick(&f, val, &c.header_style)?,
            "trackColumns" => l.track_columns = id_list(&f, val, &c.track_columns)?,
            "coverShape" => l.cover_shape = pick(&f, val, &c.cover_shape)?,
            "spinRecord" => l.spin_record = flag(&f, val)?,
            "motion" => l.motion = pick(&f, val, &c.motion)?,
            _ => notes.push(format!("Left out layout.{}: Platter doesn't know it.", shown_key(k))),
        }
    }
    Ok(l)
}

/// Why a layout can't be used, or None when it can.
pub fn layout_problem(l: &Layout) -> Option<String> {
    let v = serde_json::to_value(l).ok()?;
    parse_layout(&v, &mut Vec::new()).err()
}

fn validate_look(file: &str, o: &Map<String, Value>) -> Result<ThemeFile, String> {
    let newest = CONTRACT.look.version;
    match o.get("version").and_then(Value::as_u64) {
        Some(v) if (1..=newest).contains(&v) => {}
        Some(v) if v > newest => return Err(format!("This look was made for a newer Platter (format {v}). Update Platter to use it.")),
        _ => return Err(format!("The version must be a whole number from 1 to {newest}.")),
    }
    let mut notes = Vec::new();
    for k in o.keys().filter(|k| !LOOK_FIELDS.contains(&k.as_str())) {
        notes.push(format!("Left out \"{}\": Platter doesn't know it.", shown_key(k)));
    }
    let (name, author, base) = header(o)?;
    let mut tokens = BTreeMap::new();
    match o.get("tokens") {
        None => {}
        Some(Value::Object(t)) => {
            if t.len() > CONTRACT.tokens.len() * 2 {
                return Err("It has far more tokens than Platter knows.".into());
            }
            for (k, v) in t {
                let n = k.strip_prefix("--").unwrap_or(k);
                let Some(kind) = CONTRACT.tokens.get(n) else {
                    notes.push(format!("Left out the token \"{}\": Platter doesn't know it.", shown_key(k)));
                    continue;
                };
                if let Some(p) = value_problem(n, *kind, v) {
                    return Err(p);
                }
                tokens.insert(n.to_string(), v.as_str().unwrap_or_default().trim().to_string());
            }
        }
        Some(_) => return Err("\"tokens\" must be an object.".into()),
    }
    let style = o.get("style").map(|v| parse_style(v, &mut notes)).transpose()?;
    let fonts = o.get("fonts").map(|v| parse_fonts(v, &mut notes)).transpose()?;
    let layout = o.get("layout").map(|v| parse_layout(v, &mut notes)).transpose()?;
    Ok(ThemeFile { file: file.to_string(), name, base, tokens, author, style, fonts, layout, notes })
}

/// Check a theme or look file's text.
pub fn validate(file: &str, text: &str) -> Result<ThemeFile, String> {
    if text.len() > FILE_MAX_BYTES {
        return Err("The file is too big for a theme (64 KB at most).".into());
    }
    let data: Value = serde_json::from_str(text).map_err(|_| "It isn't valid JSON.".to_string())?;
    validate_value(file, &data)
}

/// The text of a look file: only the look itself (name, author, base,
/// tokens, style, fonts, layout), so a shared look carries nothing else.
pub fn look_text(t: &ThemeFile) -> String {
    let mut o = Map::new();
    o.insert("version".into(), Value::from(CONTRACT.look.version));
    o.insert("name".into(), Value::from(t.name.clone()));
    if let Some(a) = &t.author {
        o.insert("author".into(), Value::from(a.clone()));
    }
    o.insert("base".into(), serde_json::to_value(t.base).unwrap_or(Value::Null));
    o.insert("tokens".into(), serde_json::to_value(&t.tokens).unwrap_or_default());
    if let Some(s) = &t.style {
        o.insert("style".into(), serde_json::to_value(s).unwrap_or_default());
    }
    if let Some(f) = &t.fonts {
        o.insert("fonts".into(), serde_json::to_value(f).unwrap_or_default());
    }
    if let Some(l) = &t.layout {
        o.insert("layout".into(), serde_json::to_value(l).unwrap_or_default());
    }
    serde_json::to_string_pretty(&Value::Object(o)).unwrap_or_default()
}

/// Check a theme held in settings (it may have come through the sync file).
/// A look held in settings must have nothing left out.
pub fn check(t: &ThemeFile) -> Result<(), String> {
    if !t.is_look() {
        let data = serde_json::json!({ "name": t.name, "base": t.base, "tokens": t.tokens });
        return validate_value(&t.file, &data).map(|_| ());
    }
    let checked = validate(&t.file, &look_text(t))?;
    if let Some(n) = checked.notes.first() {
        return Err(n.clone());
    }
    if checked.tokens != t.tokens {
        return Err("Its tokens need to be plain names like color-accent.".into());
    }
    Ok(())
}

fn is_theme_path(p: &Path) -> bool {
    p.is_file() && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json") || x.eq_ignore_ascii_case(LOOK_EXT))
}

/// Every .json and .platter-look file in the folder, checked, sorted by file name.
pub fn list_dir(dir: &Path) -> ThemeFolder {
    let mut themes = Vec::new();
    let mut invalid = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).filter(|p| is_theme_path(p)).collect())
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

/// A file name stem for a theme called `name`.
fn free_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') { c } else { '_' })
        .collect::<String>()
        .trim_matches(['.', ' '])
        .to_string();
    if stem.is_empty() { "Theme".to_string() } else { stem }
}

/// A file name in `dir` for a theme called `name` that isn't taken.
fn free_name(dir: &Path, name: &str, ext: &str) -> String {
    let stem = free_stem(name);
    let mut n = 1;
    loop {
        let file = if n == 1 { format!("{stem}.{ext}") } else { format!("{stem} ({n}).{ext}") };
        if !dir.join(&file).exists() {
            return file;
        }
        n += 1;
    }
}

/// Check a theme or look file and copy it into the folder. An invalid file isn't copied.
pub fn import_into(dir: &Path, src: &Path) -> AppResult<ThemeFile> {
    let meta = std::fs::metadata(src)?;
    if meta.len() as usize > FILE_MAX_BYTES {
        return Err(AppError::Invalid("The file is too big for a theme (64 KB at most).".into()));
    }
    let text = String::from_utf8(std::fs::read(src)?).map_err(|_| AppError::Invalid("It isn't text.".into()))?;
    import_text(dir, &text)
}

/// Check a theme or look file's text and save it into the folder (a file
/// dropped on the window arrives this way). An invalid file isn't saved.
pub fn import_text(dir: &Path, text: &str) -> AppResult<ThemeFile> {
    if text.len() > FILE_MAX_BYTES {
        return Err(AppError::Invalid("The file is too big for a theme (64 KB at most).".into()));
    }
    let theme = validate("", text).map_err(AppError::Invalid)?;
    std::fs::create_dir_all(dir)?;
    let file = free_name(dir, &theme.name, if theme.is_look() { LOOK_EXT } else { "json" });
    std::fs::write(dir.join(&file), text)?;
    Ok(ThemeFile { file, ..theme })
}

/// A plain look file name in the folder: no folders, nothing hidden.
fn own_look_file(file: &str) -> bool {
    !file.is_empty() && !file.starts_with('.') && !file.contains(['/', '\\', ':']) && file.to_ascii_lowercase().ends_with(&format!(".{LOOK_EXT}"))
}

/// Save a look into the folder. With `replace` and a look file name it
/// writes over that file; otherwise it takes a free name. Returns the look
/// as saved, with its file name.
pub fn save_into(dir: &Path, look: &ThemeFile, replace: bool) -> AppResult<ThemeFile> {
    let mut look = ThemeFile { notes: Vec::new(), ..look.clone() };
    look.style.get_or_insert_with(LookStyle::default);
    check(&look).map_err(AppError::Invalid)?;
    std::fs::create_dir_all(dir)?;
    let file = if replace && own_look_file(&look.file) { look.file.clone() } else { free_name(dir, &look.name, LOOK_EXT) };
    std::fs::write(dir.join(&file), look_text(&look))?;
    Ok(ThemeFile { file, ..look })
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

/// Pick a look or theme file and copy it into the themes folder. None when cancelled.
#[tauri::command]
pub async fn themes_import(app: AppHandle) -> AppResult<Option<ThemeFile>> {
    let dir = themes_dir(&app)?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().set_title("Import a look").add_filter("Look or theme file", &[LOOK_EXT, "json"]).pick_file(move |p| {
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

/// Check the text of a look or theme file dropped on the window and add it
/// to the themes folder.
#[tauri::command]
pub async fn themes_import_text(app: AppHandle, text: String) -> AppResult<ThemeFile> {
    let dir = themes_dir(&app)?;
    tokio::task::spawn_blocking(move || import_text(&dir, &text)).await.map_err(|e| AppError::Internal(format!("import: {e}")))?
}

/// Save a look made in the builder into the themes folder.
#[tauri::command]
pub async fn themes_save(app: AppHandle, look: ThemeFile, replace: bool) -> AppResult<ThemeFile> {
    let dir = themes_dir(&app)?;
    tokio::task::spawn_blocking(move || save_into(&dir, &look, replace)).await.map_err(|e| AppError::Internal(format!("save: {e}")))?
}

/// Export a look to a file the user picks. Returns the file's name, or None
/// when the picker was cancelled.
#[tauri::command]
pub async fn themes_export(app: AppHandle, look: ThemeFile) -> AppResult<Option<String>> {
    check(&ThemeFile { notes: Vec::new(), ..look.clone() }).map_err(AppError::Invalid)?;
    let text = look_text(&look);
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_title("Export look")
        .set_file_name(format!("{}.{LOOK_EXT}", free_stem(&look.name)))
        .add_filter("Platter look", &[LOOK_EXT])
        .save_file(move |p| {
            let _ = tx.send(p);
        });
    let Some(picked) = rx.await.ok().flatten() else {
        return Ok(None);
    };
    let path = picked.into_path().map_err(|e| AppError::Io(format!("export path: {e}")))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    tokio::task::spawn_blocking(move || std::fs::write(&path, text)).await.map_err(|e| AppError::Internal(format!("export: {e}")))??;
    Ok(Some(name))
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
        #[serde(default)]
        notes: Option<usize>,
    }

    fn run_cases(json: &str, min: usize) {
        let cases: Vec<Case> = serde_json::from_str(json).unwrap();
        assert!(cases.len() >= min);
        for c in cases {
            match (validate("t.json", &c.text), c.ok) {
                (Ok(t), true) => {
                    if let Some(n) = c.notes {
                        assert_eq!(t.notes.len(), n, "{}: notes {:?}", c.case, t.notes);
                    }
                }
                (Err(r), false) => {
                    let want = c.reason.unwrap_or_default();
                    assert!(r.contains(&want), "{}: got {r:?}, want {want:?}", c.case);
                }
                (got, _) => panic!("{}: unexpected {got:?}", c.case),
            }
        }
    }

    #[test]
    fn shared_validation_cases() {
        run_cases(include_str!("../../src/design/themes/validation-cases.json"), 10);
    }

    #[test]
    fn shared_look_validation_cases() {
        run_cases(include_str!("../../src/design/themes/look-validation-cases.json"), 12);
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
        std::fs::write(dir.path().join("c.platter-look"), r#"{"version":1,"name":"C","base":"light","layout":{"sidebarSide":"right"}}"#).unwrap();
        std::fs::write(dir.path().join("notes.txt"), "not a theme").unwrap();
        std::fs::write(dir.path().join("big.json"), " ".repeat(FILE_MAX_BYTES + 1)).unwrap();
        let f = list_dir(dir.path());
        assert_eq!(f.themes.iter().map(|t| t.file.as_str()).collect::<Vec<_>>(), ["b.json", "c.platter-look"]);
        assert_eq!(f.themes[1].layout.as_ref().map(|l| l.sidebar_side.as_str()), Some("right"));
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

        // A dropped look keeps its look extension.
        let look = import_text(dest.path(), r#"{"version":1,"name":"Dusk","base":"dark","fonts":{"text":"manrope"}}"#).unwrap();
        assert_eq!(look.file, "Dusk.platter-look");
        assert!(import_text(dest.path(), r#"{"version":9,"name":"Later","base":"dark"}"#).is_err());
    }

    fn sample_look() -> ThemeFile {
        validate(
            "",
            r##"{"version":1,"name":"Dusk","author":"Sam","base":"dark","tokens":{"color-accent":"#AABBCC"},
            "style":{"density":"compact","shadow":"soft","blur":"low","border":1},
            "fonts":{"text":"atkinson","headings":"fraunces"},
            "layout":{"sidebarSide":"right","playerButtons":["queue","volume"],"quickAccess":6,"coverShape":"circle","spinRecord":true}}"##,
        )
        .unwrap()
    }

    #[test]
    fn look_round_trips_and_export_writes_only_the_look() {
        let mut look = sample_look();
        assert!(look.notes.is_empty());
        assert!(check(&look).is_ok());
        look.file = r"C:\Users\someone\secret.platter-look".into();
        let text = look_text(&look);
        let back: Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&str> = back.as_object().unwrap().keys().map(String::as_str).collect();
        for k in &keys {
            assert!(LOOK_FIELDS.contains(k), "{k}");
        }
        assert!(!text.contains("someone") && !text.contains("file"));
        let again = validate("x.platter-look", &text).unwrap();
        assert_eq!((again.style, again.fonts, again.layout), (look.style, look.fonts, look.layout));
    }

    #[test]
    fn settings_check_refuses_left_out_parts() {
        let mut look = sample_look();
        look.tokens.insert("font-family".into(), "x".into());
        assert!(check(&look).is_err());
        let mut look = sample_look();
        look.layout.as_mut().unwrap().player_buttons.push("rocket".into());
        assert!(check(&look).is_err());
        let mut l = Layout::default();
        assert!(layout_problem(&l).is_none());
        l.quick_access = 5;
        assert!(layout_problem(&l).unwrap().contains("4, 6 or 8"));
    }

    #[test]
    fn save_writes_over_only_its_own_look_file() {
        let dir = tempfile::tempdir().unwrap();
        let look = sample_look();
        let a = save_into(dir.path(), &look, false).unwrap();
        assert_eq!(a.file, "Dusk.platter-look");
        let b = save_into(dir.path(), &a, true).unwrap();
        assert_eq!(b.file, "Dusk.platter-look");
        let c = save_into(dir.path(), &a, false).unwrap();
        assert_eq!(c.file, "Dusk (2).platter-look");
        let sneaky = ThemeFile { file: "../evil.platter-look".into(), ..look.clone() };
        assert_eq!(save_into(dir.path(), &sneaky, true).unwrap().file, "Dusk (3).platter-look");
        let old = ThemeFile { file: "old.json".into(), ..look };
        assert_eq!(save_into(dir.path(), &old, true).unwrap().file, "Dusk (4).platter-look");
        assert_eq!(list_dir(dir.path()).themes.len(), 4);
    }

    #[test]
    fn classic_layout_is_the_default() {
        let l = Layout::default();
        assert_eq!((l.sidebar_side.as_str(), l.player_style.as_str(), l.header_style.as_str()), ("left", "classic", "bigArt"));
        assert_eq!(l.player_buttons.len(), CONTRACT.look.layout.player_buttons.len());
        assert_eq!(l.quick_access, 8);
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
