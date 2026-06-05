//! Pinned-window set. OS-agnostic logic; relies on [`WindowApi`] for side effects.

use std::collections::HashMap;

use anyhow::Result;

use crate::win::{WindowApi, WindowId};

/// Per-pinned-window state. Overlay handle is stored opaquely so this module
/// remains OS-agnostic and unit-testable.
pub struct PinnedEntry {
    /// Opaque handle to the overlay window (platform-specific HWND value).
    pub overlay: isize,
}

#[derive(Default)]
pub struct PinnedSet {
    map: HashMap<WindowId, PinnedEntry>,
}

impl PinnedSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn contains(&self, w: WindowId) -> bool {
        self.map.contains_key(&w)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&WindowId, &PinnedEntry)> {
        self.map.iter()
    }

    /// Pin `w` via `api`, then remember the supplied overlay/hook handles.
    /// No-op (returns `Ok(false)`) if already pinned.
    pub fn pin<A: WindowApi>(
        &mut self,
        api: &A,
        w: WindowId,
        entry: PinnedEntry,
    ) -> Result<bool> {
        if self.map.contains_key(&w) {
            return Ok(false);
        }
        api.set_topmost(w, true)?;
        self.map.insert(w, entry);
        Ok(true)
    }

    /// Unpin `w` via `api` and forget the entry. Returns the entry if removed.
    pub fn unpin<A: WindowApi>(&mut self, api: &A, w: WindowId) -> Result<Option<PinnedEntry>> {
        let Some(entry) = self.map.remove(&w) else {
            return Ok(None);
        };
        if api.is_window(w) {
            // Best-effort: even if SetWindowPos fails (target gone), still drop entry.
            let _ = api.set_topmost(w, false);
        }
        Ok(Some(entry))
    }

    /// Unpin every tracked window. Returns the removed entries so the caller
    /// can destroy overlay HWNDs and unhook events.
    pub fn drain<A: WindowApi>(&mut self, api: &A) -> Vec<(WindowId, PinnedEntry)> {
        let drained: Vec<_> = self.map.drain().collect();
        for (w, _) in &drained {
            if api.is_window(*w) {
                let _ = api.set_topmost(*w, false);
            }
        }
        drained
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::win::Rect;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeApi {
        topmost: RefCell<HashMap<WindowId, bool>>,
        existing: RefCell<HashMap<WindowId, bool>>,
        fail_next: RefCell<bool>,
    }

    impl FakeApi {
        #[allow(dead_code)]
        fn with_window(self, w: WindowId) -> Self {
            self.existing.borrow_mut().insert(w, true);
            self
        }
    }

    impl WindowApi for FakeApi {
        fn set_topmost(&self, w: WindowId, on: bool) -> Result<()> {
            if *self.fail_next.borrow() {
                *self.fail_next.borrow_mut() = false;
                anyhow::bail!("forced failure");
            }
            self.topmost.borrow_mut().insert(w, on);
            Ok(())
        }
        fn window_rect(&self, _w: WindowId) -> Result<Rect> {
            Ok(Rect::default())
        }
        fn is_window(&self, w: WindowId) -> bool {
            *self.existing.borrow().get(&w).unwrap_or(&true)
        }
    }

    fn entry() -> PinnedEntry {
        PinnedEntry { overlay: 0 }
    }

    #[test]
    fn pin_then_unpin_round_trips() {
        let api = FakeApi::default();
        let mut set = PinnedSet::new();
        let w = WindowId(0xABCD);

        assert!(set.pin(&api, w, entry()).unwrap());
        assert!(set.contains(w));
        assert_eq!(api.topmost.borrow().get(&w), Some(&true));

        let removed = set.unpin(&api, w).unwrap();
        assert!(removed.is_some());
        assert!(!set.contains(w));
        assert_eq!(api.topmost.borrow().get(&w), Some(&false));
    }

    #[test]
    fn double_pin_is_noop() {
        let api = FakeApi::default();
        let mut set = PinnedSet::new();
        let w = WindowId(1);
        assert!(set.pin(&api, w, entry()).unwrap());
        assert!(!set.pin(&api, w, entry()).unwrap());
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn pin_failure_does_not_insert() {
        let api = FakeApi::default();
        *api.fail_next.borrow_mut() = true;
        let mut set = PinnedSet::new();
        assert!(set.pin(&api, WindowId(2), entry()).is_err());
        assert!(set.is_empty());
    }

    #[test]
    fn unpin_unknown_is_none() {
        let api = FakeApi::default();
        let mut set = PinnedSet::new();
        assert!(set.unpin(&api, WindowId(42)).unwrap().is_none());
    }

    #[test]
    fn drain_clears_all() {
        let api = FakeApi::default();
        let mut set = PinnedSet::new();
        set.pin(&api, WindowId(1), entry()).unwrap();
        set.pin(&api, WindowId(2), entry()).unwrap();
        let drained = set.drain(&api);
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
        set.pin(&api, w, entry()).unwrap();
        api.existing.borrow_mut().insert(w, false);
        api.topmost.borrow_mut().clear();
        set.unpin(&api, w).unwrap();
        // No false-set recorded because is_window returned false.
        assert!(api.topmost.borrow().get(&w).is_none());
    }
}
