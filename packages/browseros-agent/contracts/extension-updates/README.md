# Extension update recovery

Chromium may download a Neo extension update and leave it waiting for idle while
cockpit pages or the worker remain open. The sidecar reuses its CDP connection to
read staged state and request one targeted extension reload. It never restarts the
browser or navigates web tabs.

`api.js` is the shared, self-contained browser implementation. WXT bundles it into
the extension. The Rust coordinator can also evaluate the same factory in an
older extension worker, so a sidecar release can unblock installs that have not
yet received the listener. The native `chrome.browserOS.getPref` API reads
`extensions.settings` in that worker's own profile; no filesystem/profile guesses
or preference writes are needed. Only `kWaitForIdle` (2), a newer version, and the
known Neo extension ID qualify. Other install gates retain their policy.

## Extension API

In the worker, including from CDP:

```js
await globalThis.browserosExtensionUpdates.getStatus()
// { extensionId, runningVersion, pendingVersion, detectedAt, attemptedVersion }
await globalThis.browserosExtensionUpdates.applyPendingUpdate('0.2.29.0')
// { scheduled: true } or { scheduled: false }
```

Extension pages can use the same API through runtime messaging:

```js
await chrome.runtime.sendMessage({ type: 'extension-update.getStatus' })
await chrome.runtime.sendMessage({ type: 'extension-update.apply', version: '0.2.29.0' })
// { ok: true, value: ... } or { ok: false, error: ... }
```

The message API accepts this extension's own pages, not content scripts or
external extensions. Applying requires the exact current staged version. The
attempt is persisted before reload, so duplicate calls, worker restarts, and lost
CDP replies do not produce an automatic reload loop. An uncertain/failed attempt
is reported and requires investigation; it is not blindly retried.

`runtime.onUpdateAvailable` persists the pending version and wakes the sidecar
with `POST /api/v1/extension/update-ready` (204, no request body). The notification
is only a hint: the sidecar independently reads native staged state before acting.
A newer feed version or `requestUpdateCheck()` response is not proof of readiness.

## Timing and lifecycle

The sidecar checks at startup, on notification, and every 30 seconds to recover
missed notifications. After a 30-second grace, it prefers a gap between active
MCP tool calls. Deferral ends after 14 minutes from first observation. A healthy
browser/server can therefore issue activation within 15 minutes of observing a
staged update; shutdown, disconnects, and failed installs cannot promise that SLA.
Multiple matching workers (for example, separate profiles) are skipped rather
than guessing the profile. Success requires observing the expected running
version and no pending update; a CDP acknowledgment is not success.

Before reloading, the shared API asks the old recorders to stop and flush, with a
bounded wait. Updated recorders acknowledge the worker's persistence response;
a failed or timed-out preparation resumes recording and defers activation. Legacy
recorders lack that acknowledgment, so the compatibility path sends an empty
batch from the stopped document through the existing relay. Its serialized
outbox acknowledgment is a persistence barrier and adds no replay events.
A missing acknowledgment defers activation; it never becomes a timed blind reload.

The new extension's update handler reinjects its universal content
scripts into existing eligible tabs without refreshing them. WXT invalidates old
instances; the recorder releases observers and buffers on invalidation. Extension
UI contexts restart, so in-memory cockpit state can reset. Persisted recordings
and configuration remain in their existing stores.
