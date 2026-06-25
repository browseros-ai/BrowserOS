/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-process async mutex. Tasks queued via `run` execute one at a
 * time in submission order; subsequent tasks always run, regardless
 * of whether the previous one resolved or rejected.
 *
 * Used by the agents service to serialise slug-mutating operations
 * (create, update, regenerateMcpUrl) so the read-snapshot → compute
 * → write pattern cannot race against itself. The claw server
 * is a single-process loopback bind so per-process serialisation is
 * the right granularity; multi-process scaling would warrant a
 * filesystem-level guard (O_EXCL on a lockfile) instead.
 */

export class AsyncMutex {
  private chain: Promise<unknown> = Promise.resolve()

  run<T>(task: () => Promise<T>): Promise<T> {
    // Swallow any previous rejection so the next task always runs;
    // the caller of the prior task already saw the rejection on its
    // own promise.
    const next = this.chain.catch(() => undefined).then(task)
    this.chain = next.catch(() => undefined)
    return next
  }
}
