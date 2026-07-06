/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Per-session NDJSON store for rrweb replay events. Files live at
 * `<browserclawDir>/replays/<sessionId>.ndjson`. Each line
 * is one event annotated by the recorder with `tabPageId` and `ts`;
 * the server prepends its own trusted `sessionId` before writing so
 * the on-disk shape is self-contained.
 *
 * Two reasons we hold file handles open across writes rather than
 * open-write-close per call: rrweb emits up to ~50 events per second
 * on a busy page, and 5 agents running in parallel multiply that.
 * Each handle is closed automatically after IDLE_HANDLE_MS of no
 * writes, and the open-handle set is capped at MAX_OPEN_HANDLES via
 * insertion-order LRU eviction (Map keys() iterates oldest-first).
 *
 * Concurrent appends to the same sessionId are serialised via a
 * per-key promise chain so two simultaneous writes never tear a
 * line. Different sessionIds run in parallel.
 */

import { type FileHandle, mkdir, open, stat, unlink } from 'node:fs/promises'
import { dirname } from 'node:path'
import { resolveClawServerPath } from '../lib/browserclaw-dir'
import { logger } from '../lib/logger'

export const REPLAY_DIR_NAME = 'replays'
export const MAX_OPEN_HANDLES = 50
export const IDLE_HANDLE_MS = 30_000

export interface ReplayMetadata {
  hasData: boolean
  sizeBytes: number
  firstEventAt?: number
  lastEventAt?: number
  tabPageIds: number[]
}

export interface ReplayStorage {
  appendEvents(sessionId: string, ndjsonLines: string[]): Promise<void>
  readEvents(sessionId: string): Promise<ReadableStream<Uint8Array>>
  statSession(sessionId: string): Promise<ReplayMetadata>
  deleteSession(sessionId: string): Promise<void>
  /** Test-only: forcibly close all open handles and drop the chain map. */
  resetForTesting(): Promise<void>
}

export interface ReplayStorageOptions {
  /** Root directory for replay NDJSON files; defaults to <browserclawDir>/replays */
  rootDir?: string
  maxOpenHandles?: number
  idleHandleMs?: number
}

interface OpenEntry {
  handle: FileHandle
  closeTimer: NodeJS.Timeout | null
}

export function createReplayStorage(
  opts: ReplayStorageOptions = {},
): ReplayStorage {
  const maxOpenHandles = opts.maxOpenHandles ?? MAX_OPEN_HANDLES
  const idleHandleMs = opts.idleHandleMs ?? IDLE_HANDLE_MS
  const open_ = new Map<string, OpenEntry>()
  // Per-sessionId append serialisation. Each chain holds the last
  // pending write so the next one queues behind it; without this two
  // overlapping `appendEvents` calls could fwrite half-lines.
  const chains = new Map<string, Promise<void>>()

  function resolvePath(sessionId: string): string {
    const sid = sanitiseSessionId(sessionId)
    const root = opts.rootDir ?? resolveClawServerPath(REPLAY_DIR_NAME)
    return `${root.replace(/\/$/, '')}/${sid}.ndjson`
  }

  async function evictOldestIfNeeded(): Promise<void> {
    while (open_.size > maxOpenHandles) {
      const oldestKey = open_.keys().next().value
      if (oldestKey === undefined) return
      await closeEntry(oldestKey)
    }
  }

  async function closeEntry(sessionId: string): Promise<void> {
    const entry = open_.get(sessionId)
    if (!entry) return
    open_.delete(sessionId)
    if (entry.closeTimer) clearTimeout(entry.closeTimer)
    try {
      await entry.handle.close()
    } catch (err) {
      logger.warn('replay storage close failed', {
        sessionId,
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  function bumpIdleTimer(sessionId: string): void {
    const entry = open_.get(sessionId)
    if (!entry) return
    if (entry.closeTimer) clearTimeout(entry.closeTimer)
    entry.closeTimer = setTimeout(() => {
      void closeEntry(sessionId)
    }, idleHandleMs)
    // Don't keep the event loop alive just for the idle timer.
    if (
      typeof (entry.closeTimer as { unref?: () => void }).unref === 'function'
    ) {
      ;(entry.closeTimer as { unref: () => void }).unref()
    }
  }

  async function openForAppend(sessionId: string): Promise<FileHandle> {
    const existing = open_.get(sessionId)
    if (existing) {
      // Bump to most-recent position for LRU.
      open_.delete(sessionId)
      open_.set(sessionId, existing)
      bumpIdleTimer(sessionId)
      return existing.handle
    }
    const path = resolvePath(sessionId)
    await mkdir(dirname(path), { recursive: true })
    const handle = await open(path, 'a')
    const entry: OpenEntry = { handle, closeTimer: null }
    open_.set(sessionId, entry)
    bumpIdleTimer(sessionId)
    await evictOldestIfNeeded()
    return handle
  }

  async function doAppend(sessionId: string, lines: string[]): Promise<void> {
    if (lines.length === 0) return
    const handle = await openForAppend(sessionId)
    const payload = `${lines.join('\n')}\n`
    await handle.appendFile(payload, 'utf8')
  }

  return {
    async appendEvents(sessionId, lines) {
      const prev = chains.get(sessionId) ?? Promise.resolve()
      const next = prev
        .catch(() => undefined)
        .then(() => doAppend(sessionId, lines))
      chains.set(sessionId, next)
      try {
        await next
      } finally {
        // Drop the chain entry once nothing else is queued behind us.
        if (chains.get(sessionId) === next) chains.delete(sessionId)
      }
    },
    async readEvents(sessionId) {
      const path = resolvePath(sessionId)
      const file = Bun.file(path)
      // Bun.file(path).stream() throws synchronously when the file
      // is missing. Probe existence first so the route can return
      // an empty stream uniformly without try/catch around the
      // caller. Real production reads go through the route which
      // checks statSession first anyway.
      if (!(await file.exists())) {
        return new ReadableStream<Uint8Array>({
          start(controller) {
            controller.close()
          },
        })
      }
      return file.stream()
    },
    async statSession(sessionId) {
      const path = resolvePath(sessionId)
      let st: Awaited<ReturnType<typeof stat>>
      try {
        st = await stat(path)
      } catch {
        return { hasData: false, sizeBytes: 0, tabPageIds: [] }
      }
      if (st.size === 0) {
        return { hasData: false, sizeBytes: 0, tabPageIds: [] }
      }
      // Read the first line for firstEventAt and the last for lastEventAt.
      // The full file may be large so we read at most HEAD_BYTES + TAIL_BYTES
      // and walk to the first/last full line within those windows.
      const HEAD_BYTES = 4096
      const TAIL_BYTES = 4096
      const fh = await open(path, 'r')
      try {
        const headLen = Math.min(HEAD_BYTES, st.size)
        const headBuf = Buffer.alloc(headLen)
        await fh.read(headBuf, 0, headLen, 0)
        const firstNewline = headBuf.indexOf(0x0a)
        const firstLine =
          firstNewline === -1
            ? headBuf.toString('utf8')
            : headBuf.subarray(0, firstNewline).toString('utf8')
        const tailLen = Math.min(TAIL_BYTES, st.size)
        const tailBuf = Buffer.alloc(tailLen)
        await fh.read(tailBuf, 0, tailLen, st.size - tailLen)
        const tailStr = tailBuf.toString('utf8')
        const tailLines = tailStr.split('\n').filter(Boolean)
        const lastLine = tailLines[tailLines.length - 1] ?? firstLine
        const firstEventAt = readTs(firstLine)
        const lastEventAt = readTs(lastLine)
        // For tabPageIds we sample the head + tail; a more thorough
        // scan is reserved for a future enhancement when a session
        // genuinely spans many tabs and the first/last windows miss
        // some of them.
        const tabIds = new Set<number>()
        for (const line of [firstLine, ...tailLines]) {
          const id = readTabPageId(line)
          if (id !== null) tabIds.add(id)
        }
        return {
          hasData: true,
          sizeBytes: st.size,
          firstEventAt,
          lastEventAt,
          tabPageIds: [...tabIds].sort((a, b) => a - b),
        }
      } finally {
        await fh.close()
      }
    },
    async deleteSession(sessionId) {
      await closeEntry(sessionId)
      const path = resolvePath(sessionId)
      try {
        await unlink(path)
      } catch (err: unknown) {
        const code = (err as { code?: string } | null)?.code
        if (code !== 'ENOENT') {
          logger.warn('replay storage delete failed', {
            sessionId,
            error: err instanceof Error ? err.message : String(err),
          })
        }
      }
    },
    async resetForTesting() {
      for (const key of [...open_.keys()]) await closeEntry(key)
      chains.clear()
      // Best-effort delete the rootDir so subsequent tests see a
      // clean directory. Ignore errors (the dir may not exist).
      if (opts.rootDir) {
        try {
          const { rm } = await import('node:fs/promises')
          await rm(opts.rootDir, { recursive: true, force: true })
        } catch {
          // ignore
        }
      }
    },
  }
}

function sanitiseSessionId(sessionId: string): string {
  // sessionIds are UUIDs in production but we still defend against a
  // path-traversal attempt. Keep alnum + dash + underscore + dot only.
  return sessionId.replace(/[^A-Za-z0-9._-]/g, '_')
}

function readTs(line: string): number | undefined {
  try {
    const obj = JSON.parse(line)
    const ts = (obj as { ts?: number }).ts
    return typeof ts === 'number' ? ts : undefined
  } catch {
    return undefined
  }
}

function readTabPageId(line: string): number | null {
  try {
    const obj = JSON.parse(line)
    const id = (obj as { tabPageId?: number }).tabPageId
    return typeof id === 'number' ? id : null
  } catch {
    return null
  }
}

/** Process-wide singleton used by routes + the injection hook. */
export const replayStorage = createReplayStorage()
