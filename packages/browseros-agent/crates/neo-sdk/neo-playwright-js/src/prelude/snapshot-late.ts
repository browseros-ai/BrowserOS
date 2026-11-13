/**
 * A whole-page v1 snapshot whose call's timeout passes after its tree arrived, and the one place
 * that chooses what the call then does.
 *
 *   page.ariaSnapshot          the tree; the op's own timeout in Rust judges it
 *   page.snapshotBaseline      (a delta) the tab's stored text            ┐ the finalization:
 *   page.snapshotPending       the text the call returns, as pending      ┘ baseline ops
 *
 * The call's timeout covers the baseline ops too. Whether it has passed is Rust's decision, made
 * from the tree op's own record of the call (neo-playwright `snapshot_calls.rs`): a baseline op
 * sent after it fails as that call's `website / wait_timeout`, naming the phase, with its own op
 * id. The realm never raises that timeout itself: the script shares the realm, so a timeout it
 * could raise is one a script could forge (`new TimeoutError(...)`, or copied fields), which must
 * stay the script's own error.
 *
 * `LATE_SNAPSHOT` is the owner's choice between two defensible answers:
 * - `'return'` (A): the snapshot the call holds is returned, and no baseline is recorded, so the
 *   next delta compares against older text the caller holds, or comes back whole. With no time
 *   left by the realm's clock, no baseline op is sent at all; one Rust times out is dropped.
 * - `'fail'` (B): the call fails with the timeout Rust decided, as the deadline covers the whole
 *   call. Every baseline op is sent, so Rust decides whether the time is gone.
 */

import type { EventDeadline } from './event-deadline'

/** The owner's choice (see above). Tests flip it in a fresh build to check B. */
export const LATE_SNAPSHOT: LateSnapshot = 'return'

export type LateSnapshot = 'return' | 'fail'

/**
 * One baseline op of a whole-page v1 snapshot whose tree arrived, sent with what is left of the
 * call's deadline: its value, or null under A when it was skipped for want of time or Rust timed
 * it out, for the call to return its snapshot. Any other failure, an abort through the script's
 * signal included, is the op's own and fails the call, as Rust's timeout does under B.
 */
export async function baselineOp<T>(
  late: LateSnapshot,
  deadline: EventDeadline,
  op: (timeoutMs: number) => Promise<T>,
): Promise<{ value: T } | null> {
  const left = deadline.remaining()
  // B: a call the realm thinks is out of time still asks, and Rust's own clock decides. The op's
  // `timeoutMs` 0 only means it has no budget of its own.
  if (late === 'fail') return { value: await op(left ?? 0) }
  if (left === null) return null
  try {
    return { value: await op(left) }
  } catch (error) {
    if (error instanceof Error && Reflect.get(error, 'code') === 'wait_timeout')
      return null
    throw error
  }
}
