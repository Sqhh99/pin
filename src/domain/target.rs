//! What a badge should do about its target, given the target's current state.
//!
//! Many apps "exit" to the tray by hiding their main window rather than
//! destroying it. A hidden target is treated as closed — the pin is dropped —
//! once it has stayed hidden for [`HIDDEN_GRACE`], so a brief hide/show
//! flicker doesn't cost the user their pin. Minimized and cloaked targets
//! (e.g. on another virtual desktop) keep their pin; only the badge hides.

use std::time::{Duration, Instant};

/// How long a target may stay hidden before it counts as closed.
pub const HIDDEN_GRACE: Duration = Duration::from_millis(500);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetStatus {
    /// The window was destroyed.
    Gone,
    /// The window exists but is not visible (e.g. closed to the tray).
    Hidden,
    Minimized,
    /// Visible to USER but not drawn by DWM, e.g. on another virtual desktop.
    Cloaked,
    Shown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeAction {
    /// Drop the pin and the badge.
    Unpin,
    /// Keep the pin; hide the badge for now.
    Hide,
    /// Keep the badge on the target's caption.
    Track,
}

/// Remembers when the target was first seen hidden, to debounce
/// [`TargetStatus::Hidden`].
#[derive(Clone, Copy, Debug, Default)]
pub struct TargetWatch {
    hidden_since: Option<Instant>,
}

impl TargetWatch {
    pub fn observe(&mut self, status: TargetStatus, now: Instant) -> BadgeAction {
        if status != TargetStatus::Hidden {
            self.hidden_since = None;
        }
        match status {
            TargetStatus::Gone => BadgeAction::Unpin,
            TargetStatus::Hidden => {
                let since = *self.hidden_since.get_or_insert(now);
                if now.duration_since(since) >= HIDDEN_GRACE {
                    BadgeAction::Unpin
                } else {
                    BadgeAction::Hide
                }
            }
            TargetStatus::Minimized | TargetStatus::Cloaked => BadgeAction::Hide,
            TargetStatus::Shown => BadgeAction::Track,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TICK: Duration = Duration::from_millis(16);

    #[test]
    fn gone_unpins_immediately() {
        let mut w = TargetWatch::default();
        assert_eq!(
            w.observe(TargetStatus::Gone, Instant::now()),
            BadgeAction::Unpin
        );
    }

    #[test]
    fn shown_tracks() {
        let mut w = TargetWatch::default();
        assert_eq!(
            w.observe(TargetStatus::Shown, Instant::now()),
            BadgeAction::Track
        );
    }

    #[test]
    fn minimized_and_cloaked_only_hide() {
        let mut w = TargetWatch::default();
        let t0 = Instant::now();
        for status in [TargetStatus::Minimized, TargetStatus::Cloaked] {
            assert_eq!(w.observe(status, t0), BadgeAction::Hide);
            // No matter how long it lasts.
            assert_eq!(
                w.observe(status, t0 + HIDDEN_GRACE * 100),
                BadgeAction::Hide
            );
        }
    }

    #[test]
    fn hidden_unpins_after_grace() {
        let mut w = TargetWatch::default();
        let t0 = Instant::now();
        assert_eq!(w.observe(TargetStatus::Hidden, t0), BadgeAction::Hide);
        assert_eq!(
            w.observe(TargetStatus::Hidden, t0 + HIDDEN_GRACE - TICK),
            BadgeAction::Hide
        );
        assert_eq!(
            w.observe(TargetStatus::Hidden, t0 + HIDDEN_GRACE),
            BadgeAction::Unpin
        );
    }

    #[test]
    fn brief_hide_keeps_pin() {
        let mut w = TargetWatch::default();
        let t0 = Instant::now();
        assert_eq!(w.observe(TargetStatus::Hidden, t0), BadgeAction::Hide);
        assert_eq!(
            w.observe(TargetStatus::Shown, t0 + TICK),
            BadgeAction::Track
        );
        // The grace period restarts from the next hide.
        let t1 = t0 + HIDDEN_GRACE;
        assert_eq!(w.observe(TargetStatus::Hidden, t1), BadgeAction::Hide);
        assert_eq!(
            w.observe(TargetStatus::Hidden, t1 + HIDDEN_GRACE - TICK),
            BadgeAction::Hide
        );
    }

    #[test]
    fn minimized_then_hidden_starts_grace_at_hide() {
        // Minimize-to-tray apps minimize first, then hide.
        let mut w = TargetWatch::default();
        let t0 = Instant::now();
        assert_eq!(w.observe(TargetStatus::Minimized, t0), BadgeAction::Hide);
        let t1 = t0 + HIDDEN_GRACE * 2;
        assert_eq!(w.observe(TargetStatus::Hidden, t1), BadgeAction::Hide);
        assert_eq!(
            w.observe(TargetStatus::Hidden, t1 + HIDDEN_GRACE),
            BadgeAction::Unpin
        );
    }
}
