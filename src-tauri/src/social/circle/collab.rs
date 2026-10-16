//! Collaborative playlists: the rules, apart from the transport.
//!
//! The playlist is a normal Plex playlist on its owner's server. The owner's
//! Platter is the only one that changes it: a collaborator's add or remove
//! travels to the owner ([`super::wire::Msg::Edit`]), which checks it
//! ([`check_edit`]), applies it to Plex, and sends everyone the change.
//! Edits are applied in the order the owner gets them; the owner's playlist
//! is what everyone ends up with.

use std::collections::HashMap;

use super::wire::{Delta, EditOp, Entry, Ident, Role};

/// Songs one edit may add.
pub const MAX_ADD: usize = 100;
/// Songs one shared playlist may have (what members are sent).
pub const MAX_SONGS: usize = 2000;

/// What the owner's playlist holds now, against what members were last
/// sent: the new list, and the change (None: nothing changed). A row that
/// went and came back with the same song in one go (a local row that got
/// its Plex id) keeps who added it. New rows whose song a collaborator
/// asked for (`claims`, used up here) are theirs.
pub fn diff(
    old: &[Entry],
    v: u64,
    rows: &[(String, String)],
    who: &HashMap<String, Ident>,
    claims: &mut Vec<(String, String)>,
    now_s: i64,
) -> (Vec<Entry>, Option<Delta>) {
    let rows = &rows[..rows.len().min(MAX_SONGS)];
    let by_key: HashMap<&str, &Entry> = old.iter().map(|e| (e.key.as_str(), e)).collect();
    let keys_now: std::collections::HashSet<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    // Rows that are going, by song, to hand their "added by" on.
    let mut going: HashMap<String, Vec<&Entry>> = HashMap::new();
    for e in old.iter().filter(|e| !keys_now.contains(e.key.as_str())) {
        going.entry(e.s.k.clone().unwrap_or_default()).or_default().push(e);
    }
    let mut list = Vec::with_capacity(rows.len());
    let mut add = Vec::new();
    for (key, track) in rows {
        if let Some(e) = by_key.get(key.as_str()) {
            list.push((*e).clone());
            continue;
        }
        let s = who.get(track).cloned().unwrap_or_else(|| Ident { k: Some(track.clone()), ..Default::default() });
        let (by, at) = match going.get_mut(track).and_then(|v| if v.is_empty() { None } else { Some(v.remove(0)) }) {
            Some(prev) => (prev.by.clone(), prev.at),
            None => match claims.iter().position(|(t, _)| t == track) {
                Some(i) => (Some(claims.remove(i).1), now_s),
                None => (None, now_s),
            },
        };
        let e = Entry { key: key.clone(), s, by, at };
        add.push(e.clone());
        list.push(e);
    }
    let del: Vec<String> = old.iter().filter(|e| !keys_now.contains(e.key.as_str())).map(|e| e.key.clone()).collect();
    let kept_old: Vec<&str> = old.iter().filter(|e| keys_now.contains(e.key.as_str())).map(|e| e.key.as_str()).collect();
    let kept_new: Vec<&str> = list.iter().filter(|e| by_key.contains_key(e.key.as_str())).map(|e| e.key.as_str()).collect();
    let reordered = kept_old != kept_new || {
        // New rows anywhere but the end move the rest.
        let firsts: Vec<&str> = list.iter().map(|e| e.key.as_str()).take(kept_new.len()).collect();
        firsts != kept_new
    };
    if add.is_empty() && del.is_empty() && !reordered {
        return (list, None);
    }
    let order = reordered.then(|| list.iter().map(|e| e.key.clone()).collect());
    (list, Some(Delta { from: v, v: v + 1, add, del, order }))
}

/// Apply a change to a member's copy. False when it doesn't follow on from
/// what they have (they ask for the whole playlist instead).
pub fn apply(songs: &mut Vec<Entry>, v: &mut u64, d: &Delta) -> bool {
    if d.from != *v {
        return false;
    }
    songs.retain(|e| !d.del.contains(&e.key));
    for e in &d.add {
        if !songs.iter().any(|x| x.key == e.key) {
            songs.push(e.clone());
        }
    }
    if let Some(order) = &d.order {
        let pos: HashMap<&str, usize> = order.iter().enumerate().map(|(i, k)| (k.as_str(), i)).collect();
        songs.sort_by_key(|e| pos.get(e.key.as_str()).copied().unwrap_or(usize::MAX));
    }
    *v = d.v;
    true
}

/// May this member make this edit? Err: why not, for them.
pub fn check_edit(role: Role, me: &str, op: &EditOp, songs: &[Entry]) -> Result<(), String> {
    if role != Role::Collaborator {
        return Err("Only collaborators can change this playlist.".into());
    }
    match op {
        EditOp::Add { songs: s } if s.is_empty() || s.len() > MAX_ADD => Err("Those songs couldn't be added.".into()),
        EditOp::Add { .. } => Ok(()),
        EditOp::Remove { keys } => {
            for k in keys {
                match songs.iter().find(|e| &e.key == k) {
                    // Gone already (someone else removed it first): nothing to do.
                    None => {}
                    Some(e) if e.by.as_deref() == Some(me) => {}
                    Some(_) => return Err("You can only remove songs you added.".into()),
                }
            }
            Ok(())
        }
    }
}

/// How many songs each person added.
pub fn added_counts(songs: &[Entry]) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    for e in songs {
        if let Some(b) = &e.by {
            *out.entry(b.clone()).or_insert(0) += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(ids: &[&str]) -> HashMap<String, Ident> {
        ids.iter().map(|i| (i.to_string(), Ident { n: format!("Song {i}"), a: "A".into(), k: Some(i.to_string()), ..Default::default() })).collect()
    }

    fn rows(r: &[(&str, &str)]) -> Vec<(String, String)> {
        r.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn changes_go_out_as_deltas_and_members_follow_them() {
        let w = who(&["t:1", "t:2", "t:3"]);
        let mut claims = vec![("t:3".to_string(), "jules".to_string())];
        let (a, d) = diff(&[], 0, &rows(&[("i1", "t:1"), ("i2", "t:2")]), &w, &mut claims, 10);
        let d = d.unwrap();
        assert_eq!((d.from, d.v, d.add.len()), (0, 1, 2));
        let mut member: Vec<Entry> = Vec::new();
        let mut mv = 0;
        assert!(apply(&mut member, &mut mv, &d));
        // Jules's song comes in at the end; the owner removes one.
        let (b, d2) = diff(&a, 1, &rows(&[("i2", "t:2"), ("i3", "t:3")]), &w, &mut claims, 20);
        let d2 = d2.unwrap();
        assert_eq!(d2.del, vec!["i1".to_string()]);
        assert_eq!(d2.add[0].by.as_deref(), Some("jules"));
        assert!(claims.is_empty());
        assert!(apply(&mut member, &mut mv, &d2));
        assert_eq!(member, b);
        // Nothing changed: nothing to send.
        assert!(diff(&b, 2, &rows(&[("i2", "t:2"), ("i3", "t:3")]), &w, &mut claims, 30).1.is_none());
        // A member out of step doesn't apply it.
        let mut stale = 0;
        assert!(!apply(&mut member.clone(), &mut stale, &d2));
    }

    #[test]
    fn a_reorder_sends_the_order_and_a_new_id_keeps_who_added_it() {
        let w = who(&["t:1", "t:2"]);
        let old = vec![
            Entry { key: "i1".into(), s: w["t:1"].clone(), by: None, at: 1 },
            Entry { key: "local-2".into(), s: w["t:2"].clone(), by: Some("jules".into()), at: 2 },
        ];
        let (list, d) = diff(&old, 4, &rows(&[("i9", "t:2"), ("i1", "t:1")]), &w, &mut Vec::new(), 9);
        let d = d.unwrap();
        assert_eq!(d.order.as_deref(), Some(&["i9".to_string(), "i1".to_string()][..]));
        assert_eq!((list[0].by.as_deref(), list[0].at), (Some("jules"), 2));
        let mut m = old.clone();
        let mut v = 4;
        assert!(apply(&mut m, &mut v, &d));
        assert_eq!(m.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(), vec!["i9", "i1"]);
    }

    #[test]
    fn collaborators_remove_only_their_own_songs_and_listeners_nothing() {
        let songs = vec![
            Entry { key: "i1".into(), s: Ident::default(), by: Some("jules".into()), at: 0 },
            Entry { key: "i2".into(), s: Ident::default(), by: None, at: 0 },
        ];
        let rm = |k: &str| EditOp::Remove { keys: vec![k.into()] };
        assert!(check_edit(Role::Collaborator, "jules", &rm("i1"), &songs).is_ok());
        assert!(check_edit(Role::Collaborator, "jules", &rm("i2"), &songs).is_err());
        assert!(check_edit(Role::Collaborator, "jules", &rm("gone"), &songs).is_ok());
        assert!(check_edit(Role::Listener, "jules", &EditOp::Add { songs: vec![Ident::default()] }, &songs).is_err());
        assert!(check_edit(Role::Collaborator, "jules", &EditOp::Add { songs: vec![] }, &songs).is_err());
        assert_eq!(added_counts(&songs).get("jules"), Some(&1));
    }
}
