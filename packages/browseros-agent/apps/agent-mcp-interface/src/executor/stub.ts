/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Deterministic stub provider for `BrowserExecutor`. No real Chrome,
 * no network, no clock dependency. Used by every Phase 4+ test and by
 * Phase 2's manual smoke until the Chromium-side bridge lands.
 *
 * Observations are pure functions of the input so integration tests
 * can assert on the exact payload. Lifecycle invariants (stop
 * invalidates the handle; concurrent runs are isolated) are pinned by
 * tests/executor/stub.test.ts.
 */

import { basename } from 'node:path'
import { nanoid } from 'nanoid'
import {
  type AttachInput,
  type BrowserExecutor,
  type ClickInput,
  ExecutorRunGoneError,
  type NavigateInput,
  type Observation,
  type ReadInput,
  type RunHandle,
  type StartRunInput,
  type SubmitInput,
  type TypeInput,
} from './types'

interface InternalRun {
  readonly handle: RunHandle
  status: 'running' | 'paused' | 'stopped'
}

export class StubBrowserExecutor implements BrowserExecutor {
  private readonly runs = new Map<string, InternalRun>()

  async startRun(input: StartRunInput): Promise<RunHandle> {
    const handle: RunHandle = {
      id: nanoid(8),
      agentId: input.agentId,
      task: input.task,
      site: input.site,
    }
    this.runs.set(handle.id, { handle, status: 'running' })
    return handle
  }

  async stop(run: RunHandle): Promise<void> {
    // Delete rather than mark-stopped: the executor is a process-
    // wide singleton, and the MCP layer starts a fresh ephemeral run
    // per tool call. Leaving "stopped" entries in the Map would leak
    // monotonically with traffic; deleting them is observationally
    // identical (assertAlive treats "no entry" the same as
    // "stopped").
    this.runs.delete(run.id)
  }

  async pause(run: RunHandle): Promise<void> {
    const entry = this.assertAlive(run)
    entry.status = 'paused'
  }

  async resume(run: RunHandle): Promise<void> {
    const entry = this.assertAlive(run)
    entry.status = 'running'
  }

  async navigate(run: RunHandle, input: NavigateInput): Promise<Observation> {
    this.assertAlive(run)
    return {
      verb: 'navigate',
      ok: true,
      summary: `(stub) navigated to ${input.url}`,
      detail: { url: input.url, title: 'stub page', status: 200 },
    }
  }

  async read(run: RunHandle, input: ReadInput): Promise<Observation> {
    this.assertAlive(run)
    const target = input.selector ?? 'document'
    return {
      verb: 'read',
      ok: true,
      summary: `(stub) read ${target}`,
      detail: { selector: target, text: 'lorem ipsum' },
    }
  }

  async click(run: RunHandle, input: ClickInput): Promise<Observation> {
    this.assertAlive(run)
    return {
      verb: 'click',
      ok: true,
      summary: `(stub) clicked ${input.selector}`,
      detail: { selector: input.selector },
    }
  }

  async type(run: RunHandle, input: TypeInput): Promise<Observation> {
    this.assertAlive(run)
    return {
      verb: 'type',
      ok: true,
      summary: `(stub) typed ${input.value} into ${input.selector}`,
      detail: { selector: input.selector, value: input.value },
    }
  }

  async attach(run: RunHandle, input: AttachInput): Promise<Observation> {
    this.assertAlive(run)
    const file = basename(input.filePath)
    return {
      verb: 'attach',
      ok: true,
      summary: `(stub) attached ${file} to ${input.selector}`,
      detail: { selector: input.selector, file },
    }
  }

  async submit(run: RunHandle, input: SubmitInput): Promise<Observation> {
    this.assertAlive(run)
    return {
      verb: 'submit',
      ok: true,
      summary: `(stub) submitted ${input.selector}`,
      detail: { selector: input.selector },
    }
  }

  private assertAlive(run: RunHandle): InternalRun {
    const entry = this.runs.get(run.id)
    if (!entry || entry.status === 'stopped') {
      throw new ExecutorRunGoneError(run.id)
    }
    return entry
  }
}
