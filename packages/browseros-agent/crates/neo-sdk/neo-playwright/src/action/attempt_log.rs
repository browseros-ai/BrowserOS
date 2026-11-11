//! An action attempt's call-log lines, held until its element is found.
//!
//! Upstream logs an attempt (`attempting click action`, `waiting for element to be ...`) only
//! once its locator resolved, so a locator that matches nothing logs `waiting for <locator>`
//! alone (server/frames.ts `_retryWithProgressIfNotConnected`, `continuePolling`). Neo finds the
//! element and checks it in one probe request, so a runner holds the attempt's lines until a
//! reply shows the element, and drops them when the locator still matches nothing. Once the
//! element was seen, lines go to the log at once, so later lines keep upstream's order.

use crate::op_scope::OpScope;

/// One located element's attempt lines: held until [`AttemptLog::found`], then live.
pub(crate) struct AttemptLog<'s> {
    scope: &'s OpScope<'s>,
    held: Option<Vec<String>>,
}

impl<'s> AttemptLog<'s> {
    /// A log for an element `found` already (a pinned element), or not yet.
    pub(crate) fn new(scope: &'s OpScope<'s>, found: bool) -> Self {
        Self {
            scope,
            held: (!found).then(Vec::new),
        }
    }

    /// Logs `line`, or holds it while the element is not found yet.
    pub(crate) fn log(&mut self, line: String) {
        match &mut self.held {
            Some(held) => held.push(line),
            None => self.scope.log(line),
        }
    }

    /// The element was seen: the held lines go to the log, and later ones go there at once.
    pub(crate) fn found(&mut self) {
        for line in self.held.take().unwrap_or_default() {
            self.scope.log(line);
        }
    }

    /// Whether the element was seen yet.
    pub(crate) fn is_found(&self) -> bool {
        self.held.is_none()
    }
}
