//! Whether ultrafast skills are on for this computer, and a way to hear when that changes so
//! connected agents can be told their tool list moved.

use tokio::sync::watch;

pub struct UltrafastSwitch {
    on: watch::Sender<bool>,
    /// Debug builds can pin it on for the MCP conformance suite; the plan then never turns it off.
    pinned: std::sync::atomic::AtomicBool,
}

impl Default for UltrafastSwitch {
    fn default() -> Self {
        Self {
            on: watch::Sender::new(false),
            pinned: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl UltrafastSwitch {
    #[must_use]
    pub fn is_on(&self) -> bool {
        *self.on.borrow()
    }

    /// Turns it on or off and reports whether that changed anything.
    pub fn set(&self, on: bool) -> bool {
        let on = on || self.pinned.load(std::sync::atomic::Ordering::Relaxed);
        self.on.send_if_modified(|current| {
            let changed = *current != on;
            *current = on;
            changed
        })
    }

    /// Keeps it on whatever the plan says.
    pub fn pin_on(&self) {
        self.pinned
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.set(true);
    }

    /// Wakes on every change after the call.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<bool> {
        self.on.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::UltrafastSwitch;

    #[tokio::test]
    async fn reports_changes_once_and_wakes_listeners() -> anyhow::Result<()> {
        let switch = UltrafastSwitch::default();
        let mut changes = switch.subscribe();
        assert!(!switch.is_on());
        assert!(!switch.set(false), "already off");
        assert!(switch.set(true));
        changes.changed().await?;
        assert!(*changes.borrow_and_update());
        assert!(!switch.set(true), "already on");
        assert!(!changes.has_changed()?);
        Ok(())
    }

    #[test]
    fn a_pinned_switch_stays_on() {
        let switch = UltrafastSwitch::default();
        switch.pin_on();
        assert!(!switch.set(false));
        assert!(switch.is_on());
    }
}
