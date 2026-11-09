//! Playwright: `server/dispatchers/dispatcher.ts` `DispatcherConnection.dispatch`: `entry.abortError = new AbortError(params.reason);`
//! Playwright: `server/progress.ts` `ProgressController.abort`: `this.metadata.log.push(logMessage);`
//!
//! A script's `signal` option, from the realm to the call it aborts. Playwright's client sends
//! `__abort__ {id, reason}` when a call's signal aborts, and its server aborts that call's
//! progress with `AbortError(reason)`. Neo's SDK calls the bridge's native `abort(id, reason)`
//! instead, and the call's [`Progress`](crate::progress::Progress) ends its wait the way a
//! timeout does, so every cleanup that runs for a timeout runs for an abort.
//!
//! ```text
//!  JS: controller.abort()  → the call's `abort` listener (neo-quickjs-injected/src/connection.ts)
//!    └─ __neo_runtime.abort(7, 'stop')            sync, inside the listener
//!         └─ Aborts::abort(7) → CallEntry::abort  ── reason set once, token cancelled
//!  call task: progress.race(work) ── Err(AbortError 'stop') ── reply {error: {name, message}}
//! ```
//!
//! The realm half lives here too: [`REALM_GLOBALS`] installs `AbortController` and
//! `AbortSignal`, which QuickJS lacks, before the agent's script runs.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use tokio_util::sync::CancellationToken;

use crate::progress::{CallEntry, PlaywrightError};

/// Platform: QuickJS-ng has no `AbortController`, `AbortSignal`, `EventTarget` or `Event`
/// (only `DOMException`, from `JS_AddIntrinsicAToB`), and Playwright's scripts run in Node,
/// whose classes follow WHATWG DOM "Aborting ongoing activities": `abort(reason)` defaults the
/// reason to a `DOMException` named `AbortError` with Node's message, `AbortSignal.timeout`
/// aborts with one named `TimeoutError`, and listeners run in order, each `once` one removed
/// before it runs.
///
/// Evaluates to a function that installs both globals and returns a promise that rejects with
/// the first error an `abort` listener throws. Node reports such an error as uncaught and
/// exits; the realm races the promise with the script, so the run ends with that error at
/// once, as it does for an `EventEmitter` listener's (`quickjs.rs`).
///
/// Deviation: `client/connection.ts` `Connection.sendMessageToServer`: `signal.addEventListener('abort', abortListener, { once: true });`
/// Node's signal is an `EventTarget` whose listeners get an `Event`; the realm has neither
/// class, so a listener gets a plain object with the `Event` fields a listener reads (`type`,
/// `target`, `currentTarget`, `isTrusted`, `stopImmediatePropagation`), and `dispatchEvent`
/// is not served.
pub(crate) const REALM_GLOBALS: &str = r#"() => {
  const ABORTED = 'This operation was aborted'
  const TIMED_OUT = 'The operation was aborted due to timeout'
  let fail
  const uncaught = new Promise((_, reject) => {
    fail = reject
  })
  const report = (error) => {
    const reject = fail
    fail = undefined
    if (reject) reject(error)
  }
  const signals = new WeakMap()
  const controllers = new WeakMap()
  const create = Symbol('create')
  const stateOf = (signal) => {
    const state = signals.get(signal)
    if (!state)
      throw new TypeError('Value of "this" must be of type AbortSignal')
    return state
  }
  const captureOf = (options) =>
    typeof options === 'boolean' ? options : !!(options && options.capture)
  const remove = (state, entry) => {
    entry.removed = true
    const index = state.listeners.indexOf(entry)
    if (index !== -1) state.listeners.splice(index, 1)
  }
  const fire = (signal, state) => {
    let stopped = false
    const event = {
      type: 'abort',
      target: signal,
      currentTarget: signal,
      isTrusted: true,
      bubbles: false,
      cancelable: false,
      defaultPrevented: false,
      stopImmediatePropagation() {
        stopped = true
      },
      stopPropagation() {},
      preventDefault() {},
    }
    for (const entry of [...state.listeners]) {
      if (entry.removed) continue
      if (entry.once) remove(state, entry)
      try {
        const callback = entry.callback
        if (typeof callback === 'function') callback.call(signal, event)
        else if (callback && typeof callback.handleEvent === 'function')
          callback.handleEvent(event)
      } catch (error) {
        report(error)
      }
      if (stopped) break
    }
  }
  // WHATWG's 'signal abort': every dependent (`AbortSignal.any`) takes the reason first, then
  // the signal's listeners run, then each dependent's.
  const signalAbort = (signal, reason) => {
    const state = signals.get(signal)
    if (state.aborted) return
    state.aborted = true
    state.reason = reason
    const dependents = []
    for (const dependent of state.dependents) {
      const other = signals.get(dependent)
      if (other.aborted) continue
      other.aborted = true
      other.reason = reason
      dependents.push(dependent)
    }
    state.dependents = []
    fire(signal, state)
    for (const dependent of dependents) fire(dependent, signals.get(dependent))
  }
  class AbortSignal {
    constructor(key) {
      if (key !== create) throw new TypeError('Illegal constructor')
      signals.set(this, {
        aborted: false,
        reason: undefined,
        listeners: [],
        handler: null,
        handlerEntry: null,
        dependents: [],
        // `AbortSignal.any`'s: its sources, never themselves composite (WHATWG flattens them).
        dependent: false,
        sources: [],
      })
    }
    get aborted() {
      return stateOf(this).aborted
    }
    get reason() {
      return stateOf(this).reason
    }
    throwIfAborted() {
      const state = stateOf(this)
      if (state.aborted) throw state.reason
    }
    get onabort() {
      return stateOf(this).handler
    }
    set onabort(handler) {
      const state = stateOf(this)
      const value = typeof handler === 'function' || (typeof handler === 'object' && handler !== null) ? handler : null
      state.handler = value
      if (value === null) {
        if (state.handlerEntry) remove(state, state.handlerEntry)
        state.handlerEntry = null
      } else if (!state.handlerEntry) {
        const signal = this
        state.handlerEntry = {
          callback(event) {
            const current = state.handler
            if (typeof current === 'function') current.call(signal, event)
          },
          capture: false,
          once: false,
          removed: false,
        }
        state.listeners.push(state.handlerEntry)
      }
    }
    addEventListener(type, callback, options) {
      const state = stateOf(this)
      if (callback === null || callback === undefined) return
      const capture = captureOf(options)
      const once = typeof options === 'object' && options !== null && !!options.once
      const abortOn = typeof options === 'object' && options !== null ? options.signal : undefined
      if (abortOn !== undefined && abortOn !== null && abortOn.aborted) return
      if (String(type) !== 'abort') return
      if (state.listeners.some((entry) => entry.callback === callback && entry.capture === capture)) return
      const entry = { callback, capture, once, removed: false }
      state.listeners.push(entry)
      if (abortOn !== undefined && abortOn !== null)
        abortOn.addEventListener('abort', () => remove(state, entry), { once: true })
    }
    removeEventListener(type, callback, options) {
      const state = stateOf(this)
      if (String(type) !== 'abort') return
      const capture = captureOf(options)
      const entry = state.listeners.find((entry) => entry.callback === callback && entry.capture === capture)
      if (entry) remove(state, entry)
    }
    static abort(reason) {
      const signal = new AbortSignal(create)
      const state = signals.get(signal)
      state.aborted = true
      state.reason = reason === undefined ? new DOMException(ABORTED, 'AbortError') : reason
      return signal
    }
    static timeout(delay) {
      if (typeof delay !== 'number')
        throw new TypeError('The "delay" argument must be of type number')
      if (!Number.isInteger(delay) || delay < 0 || delay > 4294967295)
        throw new RangeError('The value of "delay" is out of range. It must be an integer >= 0 && <= 4294967295')
      const signal = new AbortSignal(create)
      setTimeout(() => signalAbort(signal, new DOMException(TIMED_OUT, 'TimeoutError')), delay)
      return signal
    }
    static any(sources) {
      if (!Array.isArray(sources))
        throw new TypeError('The "signals" argument must be an instance of Array')
      const list = []
      for (const [index, source] of sources.entries()) {
        if (!signals.has(source))
          throw new TypeError(`The "signals[${index}]" argument must be an instance of AbortSignal`)
        list.push(source)
      }
      const signal = new AbortSignal(create)
      const state = signals.get(signal)
      const already = list.find((source) => signals.get(source).aborted)
      if (already) {
        state.aborted = true
        state.reason = signals.get(already).reason
        return signal
      }
      // WHATWG's 'create a dependent abort signal': a composite source contributes its own
      // sources, so every composite hangs directly off the signals that abort, and one abort
      // reaches every level of nesting with the same reason before any listener runs.
      state.dependent = true
      for (const source of list) {
        const sourceState = signals.get(source)
        const roots = sourceState.dependent ? sourceState.sources : [source]
        for (const root of roots) {
          state.sources.push(root)
          signals.get(root).dependents.push(signal)
        }
      }
      return signal
    }
  }
  Object.defineProperty(AbortSignal.prototype, Symbol.toStringTag, {
    value: 'AbortSignal',
    configurable: true,
  })
  class AbortController {
    constructor() {
      controllers.set(this, new AbortSignal(create))
    }
    get signal() {
      const signal = controllers.get(this)
      if (!signal) throw new TypeError('Value of "this" must be of type AbortController')
      return signal
    }
    abort(reason) {
      const signal = controllers.get(this)
      if (!signal) throw new TypeError('Value of "this" must be of type AbortController')
      signalAbort(signal, reason === undefined ? new DOMException(ABORTED, 'AbortError') : reason)
    }
  }
  Object.defineProperty(AbortController.prototype, Symbol.toStringTag, {
    value: 'AbortController',
    configurable: true,
  })
  for (const [name, value] of [['AbortSignal', AbortSignal], ['AbortController', AbortController]])
    Object.defineProperty(globalThis, name, { value, writable: true, configurable: true })
  return uncaught
}"#;

/// The run's calls an `abort` can still reach, by the id the SDK gave each message. A call is
/// in it from its admission until its task ends, so it holds at most the run's in-flight calls.
///
/// Neo: Playwright's `_activeProgressControllers`, keyed the same way: an id no call holds any
/// more (it replied, or never ran) is ignored, as Playwright's `__abort__` ignores one.
#[derive(Default)]
pub(crate) struct Aborts(Mutex<HashMap<u64, Arc<CallEntry>>>);

/// Neo: one call's place in [`Aborts`], removed when the call's task gives it back or ends.
pub(crate) struct Registration {
    aborts: Arc<Aborts>,
    id: u64,
    entry: Arc<CallEntry>,
}

/// Neo: the registry's two halves, used by the dispatcher (`register`) and the bridge's native
/// (`abort`). A poisoned lock (a panic while registering) reads as no call to abort.
impl Aborts {
    /// Makes `entry` reachable by `id` until the returned registration drops.
    pub(crate) fn register(self: &Arc<Self>, id: u64, entry: &Arc<CallEntry>) -> Registration {
        if let Ok(mut calls) = self.0.lock() {
            calls.insert(id, Arc::clone(entry));
        }
        Registration {
            aborts: Arc::clone(self),
            id,
            entry: Arc::clone(entry),
        }
    }

    /// The bridge's `abort(id, reason)`: aborts the call that holds `id`, if one still does.
    pub(crate) fn abort(&self, id: u64, reason: String) {
        let entry = self
            .0
            .lock()
            .ok()
            .and_then(|calls| calls.get(&id).map(Arc::clone));
        if let Some(entry) = entry {
            entry.abort(reason);
        }
    }
}

/// Neo: gives the id back, unless a later call took it over (a broken SDK reusing ids).
impl Drop for Registration {
    fn drop(&mut self) {
        if let Ok(mut calls) = self.aborts.0.lock()
            && calls
                .get(&self.id)
                .is_some_and(|held| Arc::ptr_eq(held, &self.entry))
        {
            calls.remove(&self.id);
        }
    }
}

/// One call's abort: the reason, set once, and the token its waits select on. It lives in the
/// call's [`CallEntry`], so the dispatcher's registry, the call's `Progress` and its trace record
/// all read the same one.
#[derive(Default)]
pub(crate) struct CallAbort {
    reason: OnceLock<String>,
    token: CancellationToken,
}

/// Playwright: `server/progress.ts` `ProgressController.abort`: `this._forceAbortPromise.reject(error);`
impl CallAbort {
    /// Sets the reason and cancels the token; `false` when the call was already aborted (the
    /// first reason wins, as Playwright's first `abort` decides its state).
    pub(crate) fn abort(&self, reason: String) -> bool {
        if self.reason.set(reason).is_err() {
            return false;
        }
        self.token.cancel();
        true
    }

    /// Playwright's server `AbortError` with the reason the client sent, once aborted.
    pub(crate) fn error(&self) -> Option<PlaywrightError> {
        self.reason
            .get()
            .map(|reason| PlaywrightError::aborted(reason))
    }

    /// Resolves with the call's `AbortError` once it is aborted; never otherwise.
    pub(crate) async fn aborted(&self) -> PlaywrightError {
        self.token.cancelled().await;
        // The reason is set before the token is cancelled, so it is always there.
        self.error().unwrap_or_else(|| PlaywrightError::aborted(""))
    }

    /// A token cancelled when the call aborts, for a wait that runs without the call's
    /// `Progress`. It is a child, so whoever holds it cannot abort the call itself.
    pub(crate) fn token(&self) -> CancellationToken {
        self.token.child_token()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;
    use crate::progress::{CallStart, ErrorName};

    fn entry() -> Arc<CallEntry> {
        CallEntry::for_test(CallStart {
            parent: None,
            page: None,
            shape: &[],
            timeout_ms: None,
            guid: "frame@T".to_string(),
            method: "click".to_string(),
            title: Some("Click".into()),
            params: Value::Null,
            location: None,
            input: true,
        })
    }

    #[test]
    fn the_first_reason_wins_and_names_an_abort_error() {
        let abort = CallAbort::default();
        assert!(abort.error().is_none());
        assert!(abort.abort("stop".to_string()));
        assert!(!abort.abort("later".to_string()));
        let error = abort.error().expect("aborted");
        assert_eq!(error.name, ErrorName::AbortError);
        assert_eq!(error.message, "stop");
    }

    #[test]
    fn a_child_token_follows_the_abort_but_cannot_cause_one() {
        let abort = CallAbort::default();
        let token = abort.token();
        token.cancel();
        assert!(
            abort.error().is_none(),
            "the holder of a child token cannot abort the call"
        );
        let token = abort.token();
        assert!(!token.is_cancelled());
        abort.abort("stop".to_string());
        assert!(token.is_cancelled());
    }

    #[test]
    fn an_abort_reaches_only_a_registered_call() {
        let aborts = Arc::new(Aborts::default());
        let entry = entry();
        aborts.abort(7, "early".to_string());
        let registration = aborts.register(7, &entry);
        aborts.abort(8, "other id".to_string());
        assert!(entry.abort_error().is_none());
        aborts.abort(7, "stop".to_string());
        assert_eq!(
            entry.abort_error().map(|error| error.message),
            Some("stop".to_string())
        );
        drop(registration);
        assert!(aborts.0.lock().is_ok_and(|calls| calls.is_empty()));
    }

    #[test]
    fn a_dropped_registration_leaves_a_later_holder_of_its_id_in_place() {
        let aborts = Arc::new(Aborts::default());
        let first = entry();
        let second = entry();
        let old = aborts.register(7, &first);
        let new = aborts.register(7, &second);
        drop(old);
        aborts.abort(7, "stop".to_string());
        assert!(first.abort_error().is_none());
        assert!(second.abort_error().is_some());
        drop(new);
    }
}
