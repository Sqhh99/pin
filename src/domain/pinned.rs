//! Pinned-window set. OS-agnostic logic; relies on [`WindowApi`] for side effects.

use std::collections::HashMap;

use anyhow::Result;

use crate::domain::window::{WindowApi, WindowId};

/// Tracks pinned windows together with a per-window entry `E` (on Windows,
/// the RAII overlay badge). Entries are owned by the set: removing a window
/// hands its entry back to the caller, and an entry that could not be
/// inserted is dropped — so an RAII entry can never outlive its pin.
pub struct PinnedSet<E> {
    map: HashMap<WindowId, E>,
}

impl<E> Default for PinnedSet<E> {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
        }
    }
}

impl<E> PinnedSet<E> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, w: WindowId) -> bool {
        self.map.contains_key(&w)
    }

    pub fn get(&self, w: WindowId) -> Option<&E> {
        self.map.get(&w)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Make `w` topmost via `api` and remember `entry`.
    ///
    /// Returns `Ok(false)` if `w` was already pinned. On error or no-op the
    /// entry is dropped.
    pub fn pin<A: WindowApi>(&mut self, api: &A, w: WindowId, entry: E) -> Result<bool> {
        if self.map.contains_key(&w) {
            return Ok(false);
        }
        api.set_topmost(w, true)?;
        self.map.insert(w, entry);
        Ok(true)
    }

    /// Clear topmost on `w` (if it still exists) and forget it. Returns the
    /// entry if `w` was pinned.
    pub fn unpin<A: WindowApi>(&mut self, api: &A, w: WindowId) -> Option<E> {
        let entry = self.map.remove(&w)?;
        release(api, w);
        Some(entry)
    }

    /// Unpin every tracked window and return the removed entries.
    pub fn unpin_all<A: WindowApi>(&mut self, api: &A) -> Vec<(WindowId, E)> {
        let drained: Vec<_> = self.map.drain().collect();
        for (w, _) in &drained {
            release(api, *w);
        }
        drained
    }
}

/// Best-effort: the target may already be gone, and we drop the entry either way.
fn release<A: WindowApi>(api: &A, w: WindowId) {
    if api.is_window(w) {
        let _ = api.set_topmost(w, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    #[derive(Default)]
    struct FakeApi {
        topmost: RefCell<HashMap<WindowId, bool>>,
        existing: RefCell<HashMap<WindowId, bool>>,
        fail_next: Cell<bool>,
    }

    impl WindowApi for FakeApi {
        fn set_topmost(&self, w: WindowId, on: bool) -> Result<()> {
            if self.fail_next.replace(false) {
                anyhow::bail!("forced failure");
            }
            self.topmost.borrow_mut().insert(w, on);
            Ok(())
        }
        fn is_window(&self, w: WindowId) -> bool {
            *self.existing.borrow().get(&w).unwrap_or(&true)
        }
    }

    /// Entry that counts how many times it has been dropped.
    struct DropCounter(Rc<Cell<u32>>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    #[test]
    fn pin_then_unpin_round_trips() {
        let api = FakeApi::default();
        let mut set = PinnedSet::new();
        let w = WindowId(0xABCD);

        assert!(set.pin(&api, w, ()).unwrap());
        assert!(set.contains(w));
        assert_eq!(api.topmost.borrow().get(&w), Some(&true));

        assert!(set.unpin(&api, w).is_some());
        assert!(!set.contains(w));
        assert_eq!(api.topmost.borrow().get(&w), Some(&false));
    }

    #[test]
    fn double_pin_is_noop() {
        let api = FakeApi::default();
        let mut set = PinnedSet::new();
        let w = WindowId(1);
        assert!(set.pin(&api, w, ()).unwrap());
        assert!(!set.pin(&api, w, ()).unwrap());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn pin_failure_does_not_insert_and_drops_entry() {
        let api = FakeApi::default();
        api.fail_next.set(true);
        let drops = Rc::new(Cell::new(0));
        let mut set = PinnedSet::new();
        assert!(set
            .pin(&api, WindowId(2), DropCounter(drops.clone()))
            .is_err());
        assert!(set.is_empty());
        assert_eq!(drops.get(), 1, "rejected entry must be dropped");
    }

    #[test]
    fn duplicate_pin_drops_new_entry_and_keeps_old() {
        let api = FakeApi::default();
        let first = Rc::new(Cell::new(0));
        let second = Rc::new(Cell::new(0));
        let mut set = PinnedSet::new();
        set.pin(&api, WindowId(3), DropCounter(first.clone()))
            .unwrap();
        set.pin(&api, WindowId(3), DropCounter(second.clone()))
            .unwrap();
        assert_eq!(first.get(), 0);
        assert_eq!(second.get(), 1);
    }

    #[test]
    fn unpin_unknown_is_none() {
        let api = FakeApi::default();
        let mut set: PinnedSet<()> = PinnedSet::new();
        assert!(set.unpin(&api, WindowId(42)).is_none());
    }

    #[test]
    fn unpin_all_clears_all() {
        let api = FakeApi::default();
        let mut set = PinnedSet::new();
        set.pin(&api, WindowId(1), ()).unwrap();
        set.pin(&api, WindowId(2), ()).unwrap();
        let drained = set.unpin_all(&api);
        assert_eq!(drained.len(), 2);
        assert!(set.is_empty());
        assert_eq!(api.topmost.borrow().get(&WindowId(1)), Some(&false));
        assert_eq!(api.topmost.borrow().get(&WindowId(2)), Some(&false));
    }

    #[test]
    fn unpin_skips_call_for_destroyed_window() {
        let api = FakeApi::default();
        let mut set = PinnedSet::new();
        let w = WindowId(5);
        set.pin(&api, w, ()).unwrap();
        api.existing.borrow_mut().insert(w, false);
        api.topmost.borrow_mut().clear();
        set.unpin(&api, w);
        // No false-set recorded because is_window returned false.
        assert!(api.topmost.borrow().get(&w).is_none());
    }
}
