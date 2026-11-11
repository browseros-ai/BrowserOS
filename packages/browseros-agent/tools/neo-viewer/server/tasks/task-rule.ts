/**
 * Which task a `run` call belongs to.
 *
 * MCP would make each agent connection a session, but Neo's `/mcp` is stateless (it never sends
 * `Mcp-Session-Id`), and current clients keep no session either: Claude Code 2.1 speaks the
 * 2026-07-28 revision, which has no `initialize` and nothing per-session in `_meta`. So the
 * viewer decides, in this order:
 *
 *   1. agent-task-id  the call carries `_meta["com.browseros.neo/run"].taskId`: the agent named
 *                     its task (zuzu does). Key: that id.
 *   2. process        the calling process, from the TCP connection (`peer-process.ts`): one task
 *                     per process and client name, so two Claude Codes are two tasks.
 *   3. client-idle    the process was not found: one task per client name, and a new one once
 *                     that client has made no call for `idleMs`.
 *
 * This module holds only the rule and the key → task table; finding the process and naming
 * the client happen before it, keeping a task's records happens after it. Replace this file to
 * change how calls are grouped.
 */

import type { ClientInfo, TaskIdentification } from '../../shared/api'

export const DEFAULT_IDLE_MS = 10 * 60 * 1000

/** The calling process, or why it is unknown. */
export type ProcessLookup =
  | {
      found: true
      pid: number
      startedAt: string | null
      command: string | null
    }
  | { found: false; reason: string }

/** What is known about a call when its task is chosen. */
export interface CallOrigin {
  /** The agent's own valid task id, or null. */
  agentTaskId: string | null
  client: ClientInfo
  process: ProcessLookup
  /** Epoch ms of the call. */
  at: number
}

export interface Assignment {
  taskId: string
  /** True when this call starts the task. */
  created: boolean
  /** The rule's key for the task; stored with it so a restart keeps grouping the same way. */
  key: string
  /**
   * The key of the agent behind the call (rules 2 and 3 without the idle split): a `tools/list`
   * from that agent belongs to whichever of its tasks comes next.
   */
  agentKey: string
  identifiedBy: TaskIdentification
}

interface Entry {
  taskId: string
  lastCallAt: number
  inFlight: number
}

export class TaskRule {
  private readonly entries = new Map<string, Entry>()

  constructor(
    private readonly newTaskId: (origin: CallOrigin) => string,
    private readonly idleMs = DEFAULT_IDLE_MS,
  ) {}

  /** The key of the agent behind a call, without choosing a task (for `tools/list`). */
  agentKey(client: ClientInfo, process: ProcessLookup): string {
    return process.found
      ? `process:${process.pid}:${process.startedAt ?? ''}:${client.name}`
      : `client:${client.name}`
  }

  /** Chooses the call's task and counts the call as in flight until `callEnded`. */
  assign(origin: CallOrigin): Assignment {
    const agentKey = this.agentKey(origin.client, origin.process)
    const { key, identifiedBy } = this.identify(origin, agentKey)
    let entry = this.entries.get(key)
    let created = false
    if (
      !entry ||
      (identifiedBy.kind === 'client-idle' && this.isIdle(entry, origin.at))
    ) {
      entry = {
        taskId: this.newTaskId(origin),
        lastCallAt: origin.at,
        inFlight: 0,
      }
      this.entries.set(key, entry)
      created = true
    }
    entry.inFlight++
    entry.lastCallAt = Math.max(entry.lastCallAt, origin.at)
    return { taskId: entry.taskId, created, key, agentKey, identifiedBy }
  }

  /** The call's reply came back (or it ended without one) at `at`. */
  callEnded(key: string, taskId: string, at: number): void {
    const entry = this.entries.get(key)
    if (!entry || entry.taskId !== taskId) return
    entry.inFlight = Math.max(0, entry.inFlight - 1)
    entry.lastCallAt = Math.max(entry.lastCallAt, at)
  }

  /**
   * A task read back from the log. The latest task per key wins, so after a restart the next
   * call joins it (or, for rule 3, starts a new one if it has been idle).
   */
  restore(key: string, taskId: string, lastCallAt: number): void {
    const entry = this.entries.get(key)
    if (entry && entry.lastCallAt > lastCallAt) return
    this.entries.set(key, { taskId, lastCallAt, inFlight: 0 })
  }

  private identify(
    origin: CallOrigin,
    agentKey: string,
  ): { key: string; identifiedBy: TaskIdentification } {
    if (origin.agentTaskId !== null) {
      return {
        key: `agent:${origin.agentTaskId}`,
        identifiedBy: { kind: 'agent-task-id', taskId: origin.agentTaskId },
      }
    }
    const { process } = origin
    if (process.found) {
      return {
        key: agentKey,
        identifiedBy: {
          kind: 'process',
          pid: process.pid,
          processStartedAt: process.startedAt,
          command: process.command,
        },
      }
    }
    return {
      key: agentKey,
      identifiedBy: {
        kind: 'client-idle',
        idleMinutes: this.idleMs / 60_000,
        reason: process.reason,
      },
    }
  }

  private isIdle(entry: Entry, at: number): boolean {
    return entry.inFlight === 0 && at - entry.lastCallAt > this.idleMs
  }
}
