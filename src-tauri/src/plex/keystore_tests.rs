//! The keychain cache and the macOS layout against a fake OS keychain that
//! counts reads and writes and "prompts" for items another build owns.
//! These run on every OS.

use std::collections::{HashMap, HashSet};

use super::*;

#[derive(Default)]
struct FakeOs {
    items: Mutex<HashMap<String, String>>,
    /// Items another signing identity owns: reading or updating one prompts.
    foreign: Mutex<HashSet<String>>,
    reads: Mutex<HashMap<String, usize>>,
    writes: Mutex<HashMap<String, usize>>,
    prompts: Mutex<usize>,
    deny_read: Mutex<HashSet<String>>,
    fail_write: Mutex<HashSet<String>>,
    fail_remove: Mutex<HashSet<String>>,
}

impl FakeOs {
    fn with(items: &[(&str, &str)], foreign: bool) -> Arc<Self> {
        let os = FakeOs::default();
        for (k, v) in items {
            os.items.lock().insert(k.to_string(), v.to_string());
            if foreign {
                os.foreign.lock().insert(k.to_string());
            }
        }
        Arc::new(os)
    }
    fn reads(&self, a: &str) -> usize {
        self.reads.lock().get(a).copied().unwrap_or(0)
    }
    fn writes(&self, a: &str) -> usize {
        self.writes.lock().get(a).copied().unwrap_or(0)
    }
    fn prompts(&self) -> usize {
        *self.prompts.lock()
    }
    fn has(&self, a: &str) -> bool {
        self.items.lock().contains_key(a)
    }
    fn vault(&self) -> Vault {
        serde_json::from_str(self.items.lock().get(VAULT_ACCOUNT).expect("vault item")).unwrap()
    }
    fn reset_counts(&self) {
        self.reads.lock().clear();
        self.writes.lock().clear();
        *self.prompts.lock() = 0;
    }
}

impl Backend for FakeOs {
    fn read(&self, account: &str) -> AppResult<Option<String>> {
        *self.reads.lock().entry(account.into()).or_default() += 1;
        if self.deny_read.lock().contains(account) {
            return Err(unavailable());
        }
        let v = self.items.lock().get(account).cloned();
        if v.is_some() && self.foreign.lock().contains(account) {
            *self.prompts.lock() += 1;
        }
        Ok(v)
    }
    fn write(&self, account: &str, value: &str) -> AppResult<()> {
        *self.writes.lock().entry(account.into()).or_default() += 1;
        if self.fail_write.lock().contains(account) {
            return Err(unavailable());
        }
        let existed = self.items.lock().insert(account.into(), value.into()).is_some();
        if existed && self.foreign.lock().contains(account) {
            // Updating in place keeps the old owner.
            *self.prompts.lock() += 1;
        }
        Ok(())
    }
    fn remove(&self, account: &str) -> AppResult<()> {
        if self.fail_remove.lock().contains(account) {
            return Err(unavailable());
        }
        self.items.lock().remove(account);
        self.foreign.lock().remove(account);
        Ok(())
    }
}

const LEGACY: [&str; 4] = ["plex-token", "plex-server-token", "lastfm-session", "anthropic-api-key"];

fn opts(layout: Layout, owner: Option<&str>, marker: Option<PathBuf>) -> Options {
    Options {
        layout,
        legacy: LEGACY.iter().map(|s| s.to_string()).collect(),
        owner: owner.map(str::to_string),
        marker: Arc::new(move || marker.clone()),
    }
}

/// A new run of the app: a fresh cache over the same OS keychain.
fn run(os: &Arc<FakeOs>, layout: Layout, owner: Option<&str>, marker: Option<PathBuf>) -> CachedStore {
    CachedStore::new(os.clone(), opts(layout, owner, marker))
}

fn tok(s: &str) -> Token {
    Token::new(s).unwrap()
}

fn val(store: &CachedStore, a: &str) -> Option<String> {
    store.get(a).unwrap().map(|t| t.expose().to_string())
}

// ------------------------------------------------------------ per account

#[test]
fn each_account_is_read_from_the_os_once_a_run() {
    let os = FakeOs::with(&[("plex-token", "t1")], false);
    let s = run(&os, Layout::PerAccount, None, None);
    for _ in 0..5 {
        assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
        assert_eq!(val(&s, "lastfm-session"), None);
    }
    assert_eq!(os.reads("plex-token"), 1);
    assert_eq!(os.reads("lastfm-session"), 1, "nothing stored is cached too");
}

#[test]
fn set_and_delete_keep_the_cache_current() {
    let os = FakeOs::with(&[], false);
    let s = run(&os, Layout::PerAccount, None, None);
    s.set("imgur-client-id", &tok("abc")).unwrap();
    assert_eq!(val(&s, "imgur-client-id").as_deref(), Some("abc"));
    s.delete("imgur-client-id").unwrap();
    assert_eq!(val(&s, "imgur-client-id"), None);
    assert_eq!(os.reads("imgur-client-id"), 0);
    assert!(!os.has("imgur-client-id"));
}

#[test]
fn a_refused_read_is_not_asked_again_straight_away() {
    let os = FakeOs::with(&[("plex-token", "t1")], false);
    os.deny_read.lock().insert("plex-token".into());
    let s = run(&os, Layout::PerAccount, None, None);
    assert!(s.get("plex-token").is_err());
    assert!(s.get("plex-token").is_err());
    assert_eq!(os.reads("plex-token"), 1);
}

#[test]
fn windows_layout_never_makes_a_vault() {
    let os = FakeOs::with(&[("plex-token", "t1")], true);
    let s = run(&os, Layout::PerAccount, Some("team"), None);
    assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
    assert!(!os.has(VAULT_ACCOUNT));
    assert!(os.has("plex-token"));
}

// ------------------------------------------------------------ vault

#[test]
fn the_vault_takes_in_every_older_item_and_deletes_them() {
    let os = FakeOs::with(
        &[("plex-token", "t1"), ("plex-server-token", "s1"), ("lastfm-session", "l1"), ("anthropic-api-key", "a1")],
        true,
    );
    let s = run(&os, Layout::Vault, Some("team"), None);
    assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
    // The one-time move reads each older item once.
    for a in LEGACY {
        assert_eq!(os.reads(a), 1, "{a}");
        assert!(!os.has(a), "{a} should be gone");
    }
    let v = os.vault();
    assert!(v.moved);
    assert_eq!(v.items.get("anthropic-api-key").map(String::as_str), Some("a1"));
    assert_eq!(v.items.len(), 4);
    // Everything else this run comes from memory.
    assert_eq!(val(&s, "lastfm-session").as_deref(), Some("l1"));
    assert_eq!(os.reads("lastfm-session"), 1);

    // Next launch: one read of the vault, no prompt.
    os.reset_counts();
    let s2 = run(&os, Layout::Vault, Some("team"), None);
    for (a, want) in [("plex-token", "t1"), ("plex-server-token", "s1"), ("lastfm-session", "l1"), ("anthropic-api-key", "a1")] {
        assert_eq!(val(&s2, a).as_deref(), Some(want));
    }
    assert_eq!(os.reads(VAULT_ACCOUNT), 1);
    assert_eq!(os.prompts(), 0);
    assert_eq!(os.writes(VAULT_ACCOUNT), 0);
}

#[test]
fn an_older_item_that_cant_be_deleted_stays_safe_in_the_vault() {
    let os = FakeOs::with(&[("plex-token", "t1"), ("lastfm-session", "l1")], true);
    os.fail_remove.lock().insert("plex-token".into());
    let s = run(&os, Layout::Vault, None, None);
    assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
    assert!(os.has("plex-token"), "the delete failed");
    assert!(!os.has("lastfm-session"));
    assert_eq!(os.vault().items.get("plex-token").map(String::as_str), Some("t1"));

    // Next launch reads the vault only; the leftover is never read again.
    os.reset_counts();
    let s2 = run(&os, Layout::Vault, None, None);
    assert_eq!(val(&s2, "plex-token").as_deref(), Some("t1"));
    assert_eq!(os.reads("plex-token"), 0);
    assert_eq!(os.prompts(), 0);
}

#[test]
fn a_refused_older_item_is_kept_and_tried_next_launch() {
    let os = FakeOs::with(&[("plex-token", "t1"), ("lastfm-session", "l1")], true);
    os.deny_read.lock().insert("lastfm-session".into());
    let s = run(&os, Layout::Vault, None, None);
    assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
    assert!(os.has("lastfm-session"));
    assert!(!os.vault().moved);

    os.deny_read.lock().clear();
    let s2 = run(&os, Layout::Vault, None, None);
    assert_eq!(val(&s2, "lastfm-session").as_deref(), Some("l1"));
    assert!(os.vault().moved);
    assert!(!os.has("lastfm-session"));
}

#[test]
fn entries_not_on_the_list_move_in_when_first_read() {
    let os = FakeOs::with(&[("airplay-password:abc", "pw")], true);
    let s = run(&os, Layout::Vault, None, None);
    assert_eq!(val(&s, "airplay-password:abc").as_deref(), Some("pw"));
    assert!(!os.has("airplay-password:abc"));
    assert_eq!(os.vault().items.get("airplay-password:abc").map(String::as_str), Some("pw"));
    assert_eq!(val(&s, "airplay-password:abc").as_deref(), Some("pw"));
    assert_eq!(os.reads("airplay-password:abc"), 1);
}

#[test]
fn saving_keeps_what_the_vault_already_holds() {
    let os = FakeOs::with(&[], false);
    let first = run(&os, Layout::Vault, None, None);
    first.set("plex-token", &tok("t1")).unwrap();
    let s = run(&os, Layout::Vault, None, None);
    s.set("imgur-client-id", &tok("i1")).unwrap();
    let v = os.vault();
    assert_eq!(v.items.get("plex-token").map(String::as_str), Some("t1"));
    assert_eq!(v.items.get("imgur-client-id").map(String::as_str), Some("i1"));
    s.delete("plex-token").unwrap();
    assert!(!os.vault().items.contains_key("plex-token"));
    assert_eq!(val(&s, "plex-token"), None);
}

#[test]
fn a_locked_vault_is_never_overwritten() {
    let os = FakeOs::with(&[], false);
    run(&os, Layout::Vault, None, None).set("plex-token", &tok("t1")).unwrap();
    os.deny_read.lock().insert(VAULT_ACCOUNT.into());
    let s = run(&os, Layout::Vault, None, None);
    assert!(s.set("imgur-client-id", &tok("i1")).is_err());
    os.deny_read.lock().clear();
    assert_eq!(os.vault().items.get("plex-token").map(String::as_str), Some("t1"));
}

// ------------------------------------------------------------ re-owning

fn marker_in(dir: &tempfile::TempDir) -> Option<PathBuf> {
    Some(dir.path().join("keychain-owner.json"))
}

#[test]
fn a_vault_from_another_build_is_re_saved_once() {
    let dir = tempfile::tempdir().unwrap();
    let os = FakeOs::default();
    os.items.lock().insert(VAULT_ACCOUNT.into(), r#"{"v":1,"items":{"plex-token":"t1"},"moved":true}"#.into());
    os.foreign.lock().insert(VAULT_ACCOUNT.into());
    let os = Arc::new(os);

    let s = run(&os, Layout::Vault, Some("team-a"), marker_in(&dir));
    assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
    assert_eq!(os.prompts(), 1, "one prompt for the one item");
    assert!(!os.foreign.lock().contains(VAULT_ACCOUNT), "this build owns it now");
    assert!(!os.has(STAGING_ACCOUNT));

    // Later launches: no prompt, no re-save.
    os.reset_counts();
    let s2 = run(&os, Layout::Vault, Some("team-a"), marker_in(&dir));
    assert_eq!(val(&s2, "plex-token").as_deref(), Some("t1"));
    assert_eq!(os.prompts(), 0);
    assert_eq!(os.writes(VAULT_ACCOUNT), 0);
    assert_eq!(os.writes(STAGING_ACCOUNT), 0);

    // A new signing identity re-saves once more.
    let s3 = run(&os, Layout::Vault, Some("team-b"), marker_in(&dir));
    assert_eq!(val(&s3, "plex-token").as_deref(), Some("t1"));
    assert_eq!(os.writes(VAULT_ACCOUNT), 1);
}

#[test]
fn unsigned_builds_never_re_save() {
    let dir = tempfile::tempdir().unwrap();
    let os = FakeOs::default();
    os.items.lock().insert(VAULT_ACCOUNT.into(), r#"{"v":1,"items":{"plex-token":"t1"},"moved":true}"#.into());
    let os = Arc::new(os);
    let s = run(&os, Layout::Vault, None, marker_in(&dir));
    assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
    assert_eq!(os.writes(VAULT_ACCOUNT), 0);
    assert!(!dir.path().join("keychain-owner.json").exists());
}

#[test]
fn a_failed_delete_while_re_saving_loses_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let os = FakeOs::default();
    os.items.lock().insert(VAULT_ACCOUNT.into(), r#"{"v":1,"items":{"plex-token":"t1"},"moved":true}"#.into());
    os.foreign.lock().insert(VAULT_ACCOUNT.into());
    os.fail_remove.lock().insert(VAULT_ACCOUNT.into());
    let os = Arc::new(os);

    let s = run(&os, Layout::Vault, Some("team-a"), marker_in(&dir));
    assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
    assert_eq!(os.vault().items.get("plex-token").map(String::as_str), Some("t1"));
    assert!(!os.has(STAGING_ACCOUNT), "the staged copy is cleaned up");

    // Not marked, so the next launch tries again and succeeds.
    os.fail_remove.lock().clear();
    let s2 = run(&os, Layout::Vault, Some("team-a"), marker_in(&dir));
    assert_eq!(val(&s2, "plex-token").as_deref(), Some("t1"));
    assert!(!os.foreign.lock().contains(VAULT_ACCOUNT));
}

#[test]
fn a_failed_add_while_re_saving_is_restored_from_the_staged_copy() {
    let dir = tempfile::tempdir().unwrap();
    let os = FakeOs::default();
    os.items.lock().insert(VAULT_ACCOUNT.into(), r#"{"v":1,"items":{"plex-token":"t1"},"moved":true}"#.into());
    os.foreign.lock().insert(VAULT_ACCOUNT.into());
    let os = Arc::new(os);
    os.fail_write.lock().insert(VAULT_ACCOUNT.into());

    let s = run(&os, Layout::Vault, Some("team-a"), marker_in(&dir));
    // Still usable this run from memory.
    assert_eq!(val(&s, "plex-token").as_deref(), Some("t1"));
    assert_eq!(os.writes(VAULT_ACCOUNT), 2, "tried once more");
    assert!(!os.has(VAULT_ACCOUNT));
    assert!(os.has(STAGING_ACCOUNT), "the value is still in the keychain");

    os.fail_write.lock().clear();
    let s2 = run(&os, Layout::Vault, Some("team-a"), marker_in(&dir));
    assert_eq!(val(&s2, "plex-token").as_deref(), Some("t1"));
    assert!(os.has(VAULT_ACCOUNT));
    assert!(!os.has(STAGING_ACCOUNT));
    assert!(!os.foreign.lock().contains(VAULT_ACCOUNT));
}
