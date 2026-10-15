//! The command line: `platter --play`, `--pause`, `--toggle`, `--next`,
//! `--previous`, `--like`, `--playlist <name> [--shuffle]`,
//! `--now-playing [--json]` and `--pair <code>`. It talks to the running app
//! over the local control API (automation pairing) and exits; it never
//! starts the app. The token from `--pair` is kept in the OS keychain.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use super::DECK_PORT;
use crate::plex::keychain::TokenStore;
use crate::plex::token::Token;

/// Keychain account for the command line's own token.
pub const CLI_TOKEN_ACCOUNT: &str = "automation-cli-token";
const TIMEOUT: Duration = Duration::from_secs(3);

pub const USAGE: &str = "Control the running Platter app.

  platter --pair <code>        pair once, with the code under Allow automation in Settings
  platter --play | --pause | --toggle | --next | --previous | --like
  platter --playlist <name> [--shuffle]
  platter --now-playing [--json]
";

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Pair(String),
    /// A `/v1/command` body.
    Command(serde_json::Value),
    NowPlaying { json: bool },
    Help,
}

/// Exit codes.
pub const OK: i32 = 0;
pub const FAILED: i32 = 1;
pub const USAGE_ERROR: i32 = 2;
pub const NOT_RUNNING: i32 = 3;

/// The command line's action, or None when these arguments start the app.
pub fn parse(args: &[String]) -> Option<Result<Action, String>> {
    let flags: Vec<&str> = args.iter().skip(1).map(String::as_str).collect();
    let known = ["--pair", "--play", "--pause", "--toggle", "--next", "--previous", "--like", "--playlist", "--now-playing", "--help"];
    if !flags.iter().any(|f| known.contains(f)) {
        return None;
    }
    let has = |f: &str| flags.contains(&f);
    let value = |f: &str| flags.iter().position(|x| *x == f).and_then(|i| flags.get(i + 1)).map(|s| s.to_string());
    let cmd = |c: &str| Ok(Action::Command(serde_json::json!({ "command": c })));
    Some(if has("--help") {
        Ok(Action::Help)
    } else if has("--pair") {
        match value("--pair") {
            Some(code) if code.chars().all(|c| c.is_ascii_digit()) && code.len() == 6 => Ok(Action::Pair(code)),
            _ => Err("--pair needs the six-digit code shown under Allow automation in Settings.".into()),
        }
    } else if has("--playlist") {
        match value("--playlist").filter(|v| !v.starts_with("--")) {
            Some(p) => Ok(Action::Command(serde_json::json!({ "command": "playPlaylist", "playlist": p, "shuffle": has("--shuffle") }))),
            None => Err("--playlist needs a playlist name.".into()),
        }
    } else if has("--now-playing") {
        Ok(Action::NowPlaying { json: has("--json") })
    } else if has("--play") {
        cmd("play")
    } else if has("--pause") {
        cmd("pause")
    } else if has("--toggle") {
        cmd("playPause")
    } else if has("--next") {
        cmd("next")
    } else if has("--previous") {
        cmd("previous")
    } else {
        cmd("like")
    })
}

/// One answer from the API.
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub body: serde_json::Value,
}

/// One request to 127.0.0.1:`port`. Err: the app isn't there.
pub fn request(port: u16, method: &str, path: &str, token: Option<&str>, body: Option<&serde_json::Value>) -> std::io::Result<Answer> {
    let mut s = TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, port)), TIMEOUT)?;
    s.set_read_timeout(Some(TIMEOUT))?;
    s.set_write_timeout(Some(TIMEOUT))?;
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let auth = token.map(|t| format!("Authorization: Bearer {t}\r\n")).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes())?;
    s.write_all(body.as_bytes())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    let text = String::from_utf8_lossy(&raw);
    let (head, rest) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    Ok(Answer { status, body: serde_json::from_str(rest).unwrap_or_default() })
}

/// What `--now-playing` prints without `--json`.
pub fn now_line(state: &serde_json::Value) -> String {
    let s = |k: &str| state[k].as_str().filter(|v| !v.is_empty());
    match (s("title"), s("artist")) {
        (Some(t), Some(a)) => format!("{}{t} - {a}", if state["playing"].as_bool() == Some(true) { "" } else { "(paused) " }),
        (Some(t), None) => t.to_string(),
        _ if state["private"].as_bool() == Some(true) => "Private session".into(),
        _ => "Nothing playing".into(),
    }
}

/// Run an action; returns (exit code, what to print, whether it's an error).
pub fn run_with(action: Action, port: u16, store: &dyn TokenStore) -> (i32, String) {
    let not_running = || (NOT_RUNNING, "Platter isn't running, or Allow automation is off in its Settings.".to_string());
    let token = || store.get(CLI_TOKEN_ACCOUNT).ok().flatten().map(|t| t.expose().to_owned());
    let err = |a: &Answer| a.body["error"].as_str().unwrap_or("That didn't work.").to_string();
    match action {
        Action::Help => (OK, USAGE.trim_end().into()),
        Action::Pair(code) => {
            let Ok(a) = request(port, "POST", "/v1/pair", None, Some(&serde_json::json!({ "code": code, "client": "automation" }))) else {
                return not_running();
            };
            let Some(t) = a.body["token"].as_str().filter(|_| a.status == 200) else {
                return (FAILED, err(&a));
            };
            match Token::new(t.to_owned()).and_then(|t| store.set(CLI_TOKEN_ACCOUNT, &t)) {
                Ok(()) => (OK, "Paired with Platter.".into()),
                Err(e) => (FAILED, format!("Couldn't keep the pairing in the keychain: {e}")),
            }
        }
        Action::Command(body) => {
            let Some(t) = token() else { return (FAILED, "Pair first: platter --pair <code>, with the code under Allow automation in Settings.".into()) };
            match request(port, "POST", "/v1/command", Some(&t), Some(&body)) {
                Ok(a) if a.status == 200 => (OK, String::new()),
                Ok(a) => (FAILED, err(&a)),
                Err(_) => not_running(),
            }
        }
        Action::NowPlaying { json } => {
            let Some(t) = token() else { return (FAILED, "Pair first: platter --pair <code>, with the code under Allow automation in Settings.".into()) };
            match request(port, "GET", "/v1/state?cover=none", Some(&t), None) {
                Ok(a) if a.status == 200 => {
                    let mut state = a.body;
                    if let Some(o) = state.as_object_mut() {
                        o.remove("cover");
                        o.remove("coverKey");
                    }
                    (OK, if json { state.to_string() } else { now_line(&state) })
                }
                Ok(a) => (FAILED, err(&a)),
                Err(_) => not_running(),
            }
        }
    }
}

/// Show output in the terminal that started a windowed build.
#[cfg(windows)]
fn attach_console() {
    use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
    // Fails when there's no terminal (started from a shortcut): nothing to show then.
    // SAFETY: plain Win32 call with a constant argument.
    let _ = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
}

#[cfg(not(windows))]
fn attach_console() {}

/// If these arguments are a command line action, run it and give the exit
/// code; None starts the app as usual.
pub fn main(args: &[String]) -> Option<i32> {
    let parsed = parse(args)?;
    attach_console();
    let (code, out) = match parsed {
        Ok(a) => run_with(a, DECK_PORT, &crate::plex::keychain::KeyringStore::new()),
        Err(e) => (USAGE_ERROR, format!("{e}\n\n{}", USAGE.trim_end())),
    };
    if !out.is_empty() {
        if code == OK {
            println!("{out}");
        } else {
            eprintln!("{out}");
        }
    }
    Some(code)
}
