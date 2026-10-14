//! One-at-a-time HTTP transfer gate.
//!
//! Some Plex servers allow a single concurrent file transfer: starting a
//! second body download makes the server close the older one. So every
//! body transfer (track streams, tail fetches, pre-cache downloads) holds a
//! [`Permit`] from the player's single [`TransferGate`].
//!
//! Priorities: the audible track's stream, then the decode-ahead next
//! track, then pre-caching, then background reads (the file tag scan).
//! When a better-priority request is waiting, the current holder is told to
//! yield ([`Permit::preempted`]); it drops its connection and later resumes
//! with a `Range` request.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Priority {
    /// The track that is (about to be) audible.
    Current = 0,
    /// The decode-ahead next track.
    Next = 1,
    /// Background pre-cache downloads.
    Precache = 2,
    /// Small background reads (the file tag scan). Yields to everything.
    Background = 3,
}

impl Priority {
    pub fn cell(self) -> Arc<AtomicU8> {
        Arc::new(AtomicU8::new(self as u8))
    }
}

/// Shared, changeable priority of one transfer owner (a stream gets
/// promoted from `Next` to `Current` when its track becomes audible).
pub type PriorityCell = Arc<AtomicU8>;

struct Signal {
    flag: AtomicBool,
    notify: Notify,
}

struct Holder {
    id: u64,
    prio: PriorityCell,
    signal: Arc<Signal>,
}

#[derive(Default)]
struct State {
    next_id: u64,
    holder: Option<Holder>,
    waiters: Vec<(u64, PriorityCell)>,
    /// Total preemptions (diagnostics/tests).
    preemptions: u64,
}

#[derive(Default)]
pub struct TransferGate {
    st: Mutex<State>,
    changed: Notify,
}

/// Permission to run one body transfer. Dropping it releases the gate.
pub struct Permit {
    gate: Arc<TransferGate>,
    id: u64,
    signal: Arc<Signal>,
}

impl Permit {
    /// Resolves when a better-priority transfer wants the gate.
    pub async fn preempted(&self) {
        loop {
            let n = self.signal.notify.notified();
            if self.signal.flag.load(Ordering::SeqCst) {
                return;
            }
            n.await;
        }
    }

    pub fn is_preempted(&self) -> bool {
        self.signal.flag.load(Ordering::SeqCst)
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        {
            let mut st = self.gate.st.lock();
            if st.holder.as_ref().is_some_and(|h| h.id == self.id) {
                st.holder = None;
            }
        }
        self.gate.changed.notify_waiters();
    }
}

/// Removes a waiter if its `acquire` future is dropped.
struct WaitGuard<'a> {
    gate: &'a TransferGate,
    id: u64,
}

impl Drop for WaitGuard<'_> {
    fn drop(&mut self) {
        self.gate.st.lock().waiters.retain(|w| w.0 != self.id);
        self.gate.changed.notify_waiters();
    }
}

impl TransferGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Waits for the gate. Preempts a worse-priority holder.
    pub async fn acquire(self: &Arc<Self>, prio: &PriorityCell) -> Permit {
        let id = {
            let mut st = self.st.lock();
            st.next_id += 1;
            let id = st.next_id;
            st.waiters.push((id, prio.clone()));
            id
        };
        let _guard = WaitGuard { gate: self, id };
        loop {
            let notified = self.changed.notified();
            {
                let mut st = self.st.lock();
                let mine = prio.load(Ordering::SeqCst);
                let best = st
                    .waiters
                    .iter()
                    .min_by_key(|w| (w.1.load(Ordering::SeqCst), w.0))
                    .map(|w| w.0);
                if best == Some(id) {
                    match &st.holder {
                        None => {
                            let signal = Arc::new(Signal {
                                flag: AtomicBool::new(false),
                                notify: Notify::new(),
                            });
                            st.waiters.retain(|w| w.0 != id);
                            st.holder = Some(Holder {
                                id,
                                prio: prio.clone(),
                                signal: signal.clone(),
                            });
                            return Permit {
                                gate: self.clone(),
                                id,
                                signal,
                            };
                        }
                        Some(h)
                            if h.prio.load(Ordering::SeqCst) > mine
                                && !h.signal.flag.swap(true, Ordering::SeqCst) =>
                        {
                            h.signal.notify.notify_one();
                            st.preemptions += 1;
                        }
                        _ => {}
                    }
                }
            }
            // Priorities can change without a notification; re-check often.
            let _ = tokio::time::timeout(Duration::from_millis(50), notified).await;
        }
    }

    /// Re-evaluate waiters (call after changing a priority).
    pub fn poke(&self) {
        self.changed.notify_waiters();
    }

    /// Priority of the transfer running now, if any.
    pub fn holder_priority(&self) -> Option<u8> {
        self.st
            .lock()
            .holder
            .as_ref()
            .map(|h| h.prio.load(Ordering::SeqCst))
    }

    /// Requests currently waiting for the gate.
    pub fn waiting(&self) -> usize {
        self.st.lock().waiters.len()
    }

    pub fn preemptions(&self) -> u64 {
        self.st.lock().preemptions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn one_holder_at_a_time_and_release_hands_over() {
        let g = TransferGate::new();
        let p1 = g.acquire(&Priority::Precache.cell()).await;
        let g2 = g.clone();
        let t = tokio::spawn(async move {
            let _p = g2.acquire(&Priority::Precache.cell()).await;
        });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!t.is_finished(), "equal priority must wait");
        assert!(!p1.is_preempted(), "equal priority must not preempt");
        drop(p1);
        tokio::time::timeout(Duration::from_secs(2), t).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn better_priority_preempts_worse_holder() {
        let g = TransferGate::new();
        let low = g.acquire(&Priority::Precache.cell()).await;
        let g2 = g.clone();
        let t = tokio::spawn(async move {
            let _p = g2.acquire(&Priority::Current.cell()).await;
            g2.holder_priority()
        });
        tokio::time::timeout(Duration::from_secs(2), low.preempted())
            .await
            .unwrap();
        assert_eq!(g.preemptions(), 1);
        drop(low);
        let held = tokio::time::timeout(Duration::from_secs(2), t)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(held, Some(Priority::Current as u8));
    }

    #[tokio::test]
    async fn best_waiter_wins_and_promotion_is_noticed() {
        let g = TransferGate::new();
        let hold = g.acquire(&Priority::Current.cell()).await;
        let order = Arc::new(Mutex::new(Vec::new()));
        let next_cell = Priority::Precache.cell();
        let mut tasks = Vec::new();
        for (name, cell) in [("pre", Priority::Precache.cell()), ("next", next_cell.clone())] {
            let g2 = g.clone();
            let order = order.clone();
            tasks.push(tokio::spawn(async move {
                let p = g2.acquire(&cell).await;
                order.lock().push(name);
                tokio::time::sleep(Duration::from_millis(10)).await;
                drop(p);
            }));
            // Make sure this one is queued before the next is spawned.
            while g.waiting() < tasks.len() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        }
        // "next" queued after "pre" but gets promoted before release.
        next_cell.store(Priority::Next as u8, Ordering::SeqCst);
        g.poke();
        drop(hold);
        for t in tasks {
            tokio::time::timeout(Duration::from_secs(2), t).await.unwrap().unwrap();
        }
        assert_eq!(*order.lock(), vec!["next", "pre"]);
    }

    #[tokio::test]
    async fn cancelled_waiter_does_not_block() {
        let g = TransferGate::new();
        let hold = g.acquire(&Priority::Precache.cell()).await;
        let g2 = g.clone();
        let t = tokio::spawn(async move {
            let _ = g2.acquire(&Priority::Current.cell()).await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        t.abort();
        let _ = t.await;
        drop(hold);
        let _p = tokio::time::timeout(Duration::from_secs(2), g.acquire(&Priority::Precache.cell()))
            .await
            .unwrap();
    }
}
