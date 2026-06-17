/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Browser action executor interface. The cockpit's MCP tools call
 * into this layer to actually do things in a browser; today the only
 * provider is a deterministic stub (`./stub.ts`). A future Chromium
 * provider that talks to the BrowserOS Chromium fork's local bridge
 * will be a drop-in replacement for the stub: no caller in `src/mcp/`
 * should need to change.
 *
 * Observations are DOM-first, not screenshot-based: tools return
 * structured strings + small objects so the LLM can reason cheaply
 * and the recorder (Phase 7) can replay without binary blobs.
 */

export interface RunHandle {
  readonly id: string
  readonly agentId: string
  readonly task: string
  /** Domain hint the run was scoped to (e.g. "concur.com"). Used by tools whose verb permission needs a domain but whose input doesn't carry one. */
  readonly site: string
}

export interface Observation {
  readonly verb: string
  readonly ok: boolean
  /** Single-line, human-readable summary of what happened. */
  readonly summary: string
  /** Optional structured payload for tools whose result has fields the caller will want to introspect (DOM reads, page metadata). */
  readonly detail?: Record<string, unknown>
}

export interface NavigateInput {
  readonly url: string
}

export interface ReadInput {
  readonly selector?: string
}

export interface ClickInput {
  readonly selector: string
}

export interface TypeInput {
  readonly selector: string
  readonly value: string
}

export interface AttachInput {
  readonly selector: string
  readonly filePath: string
}

export interface SubmitInput {
  readonly selector: string
}

export interface StartRunInput {
  readonly agentId: string
  readonly task: string
  readonly site: string
}

export interface BrowserExecutor {
  startRun(input: StartRunInput): Promise<RunHandle>
  stop(run: RunHandle): Promise<void>
  pause(run: RunHandle): Promise<void>
  resume(run: RunHandle): Promise<void>
  navigate(run: RunHandle, input: NavigateInput): Promise<Observation>
  read(run: RunHandle, input: ReadInput): Promise<Observation>
  click(run: RunHandle, input: ClickInput): Promise<Observation>
  type(run: RunHandle, input: TypeInput): Promise<Observation>
  attach(run: RunHandle, input: AttachInput): Promise<Observation>
  submit(run: RunHandle, input: SubmitInput): Promise<Observation>
}

/**
 * Thrown when a caller dispatches against a run handle that has
 * already been stopped. Distinct from "the input was bad" so the MCP
 * tool layer can surface "your run has expired" cleanly.
 */
export class ExecutorRunGoneError extends Error {
  readonly runId: string
  constructor(runId: string) {
    super(`executor: run ${runId} has been stopped`)
    this.name = 'ExecutorRunGoneError'
    this.runId = runId
  }
}
