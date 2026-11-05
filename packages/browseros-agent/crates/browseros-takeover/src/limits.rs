//! Every fixed number the takeover parts run with (synthesis item 8). None of them is a setting:
//! v1 has no config file, and a per-request choice belongs in the request (decision 19).

use std::time::Duration;

/// The shortest lock a host can take.
pub(crate) const MIN_LOCK_TTL: Duration = Duration::from_secs(1);

/// The longest lock, so a host that crashed cannot keep the agent off a tab for longer
/// (decision 19). The contract's `ttlSeconds` maximum is the same 1200.
pub(crate) const MAX_LOCK_TTL: Duration = Duration::from_secs(20 * 60);

/// How long taking a lock waits for the agent's in-flight writes on its tab to finish.
pub(crate) const SETTLE_BUDGET: Duration = Duration::from_secs(2);

/// How often the settle reads the tab's open writes from the trace store.
pub(crate) const SETTLE_POLL: Duration = Duration::from_millis(50);

/// How long an ended lock stays readable, for `GET /v1/locks/{id}` and an idempotent `DELETE`.
/// After that its id names nothing, so the ended list stays bounded.
pub(crate) const KEEP_ENDED_LOCK: Duration = Duration::from_secs(60 * 60);

/// Lock state changes the broadcast holds for a slow reader. The service's event log reads
/// them at once; one that falls this far behind gets `stream.reset` instead.
pub(crate) const LOCK_CHANGES: usize = 256;
