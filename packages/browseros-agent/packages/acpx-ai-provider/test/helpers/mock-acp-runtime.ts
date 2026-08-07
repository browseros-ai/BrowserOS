import type {
  AcpRuntime,
  AcpRuntimeCapabilities,
  AcpRuntimeDoctorReport,
  AcpRuntimeEnsureInput,
  AcpRuntimeEvent,
  AcpRuntimeHandle,
  AcpRuntimeStatus,
  AcpRuntimeTurn,
  AcpRuntimeTurnInput,
  AcpRuntimeTurnResult,
} from 'acpx/runtime'

export interface MockTurnScript {
  /** Events the turn iterator will yield in order. */
  events: AcpRuntimeEvent[]
  /** Terminal result resolved on `turn.result`. Defaults to a clean `end_turn` completion. */
  result?: AcpRuntimeTurnResult
  /** Reject `turn.result` with this error instead of resolving. */
  resultError?: Error
  /** Throw synchronously when `startTurn(...)` is called for this turn. */
  startTurnError?: Error
  /** Optional ms delay between yielded events (useful for simulating streaming). */
  delayMsBetweenEvents?: number
}

export interface MockAcpRuntimeOptions {
  /** Scripts consumed in order; missing entries default to a single completed turn with no events. */
  turnScripts?: MockTurnScript[]
  /** Reject `ensureSession(...)` with this error. */
  ensureSessionError?: Error
  /** Override the handle returned by `ensureSession(...)`. */
  ensureSessionHandle?: (input: AcpRuntimeEnsureInput) => AcpRuntimeHandle
  capabilities?: AcpRuntimeCapabilities
  status?: AcpRuntimeStatus
  doctorReport?: AcpRuntimeDoctorReport
}

const DEFAULT_RESULT: AcpRuntimeTurnResult = {
  status: 'completed',
  stopReason: 'end_turn',
}

const DEFAULT_DOCTOR: AcpRuntimeDoctorReport = {
  ok: true,
  message: 'mock runtime ready',
}

const DEFAULT_CAPABILITIES: AcpRuntimeCapabilities = {
  controls: ['session/set_mode', 'session/set_config_option', 'session/status'],
}

const DEFAULT_STATUS: AcpRuntimeStatus = { summary: 'mock' }

export interface RecordedTurn {
  input: AcpRuntimeTurnInput
  cancelCalls: Array<{ reason?: string }>
  closeStreamCalls: Array<{ reason?: string }>
}

/**
 * In-memory `AcpRuntime` modeled on AI SDK's `MockLanguageModelV3`. Records
 * every method call so tests can assert on lifecycle, and yields scripted
 * event sequences from `startTurn(...)` / `runTurn(...)`.
 */
export class MockAcpRuntime implements AcpRuntime {
  readonly ensureSessionCalls: AcpRuntimeEnsureInput[] = []
  readonly startTurnCalls: AcpRuntimeTurnInput[] = []
  readonly turns: RecordedTurn[] = []
  readonly cancelCalls: Array<{ handle: AcpRuntimeHandle; reason?: string }> =
    []
  readonly closeCalls: Array<{
    handle: AcpRuntimeHandle
    reason: string
    discardPersistentState?: boolean
  }> = []
  readonly setModeCalls: Array<{ handle: AcpRuntimeHandle; mode: string }> = []
  readonly setConfigOptionCalls: Array<{
    handle: AcpRuntimeHandle
    key: string
    value: string
  }> = []
  readonly getStatusCalls: Array<{ handle: AcpRuntimeHandle }> = []
  readonly getCapabilitiesCalls: Array<{ handle?: AcpRuntimeHandle }> = []
  readonly doctorCalls: number[] = []

  private readonly opts: MockAcpRuntimeOptions
  private turnIndex = 0
  private requestIdSeq = 0

  constructor(opts: MockAcpRuntimeOptions = {}) {
    this.opts = opts
  }

  ensureSession(input: AcpRuntimeEnsureInput): Promise<AcpRuntimeHandle> {
    this.ensureSessionCalls.push(input)
    if (this.opts.ensureSessionError) {
      return Promise.reject(this.opts.ensureSessionError)
    }
    const handle =
      this.opts.ensureSessionHandle?.(input) ?? defaultHandle(input)
    return Promise.resolve(handle)
  }

  startTurn(input: AcpRuntimeTurnInput): AcpRuntimeTurn {
    this.startTurnCalls.push(input)
    const script = this.opts.turnScripts?.[this.turnIndex]
    this.turnIndex += 1

    if (script?.startTurnError) throw script.startTurnError

    const recorded: RecordedTurn = {
      input,
      cancelCalls: [],
      closeStreamCalls: [],
    }
    this.turns.push(recorded)

    const requestId = `req-${++this.requestIdSeq}`
    const events = script?.events ?? []
    const result = script?.result ?? DEFAULT_RESULT
    const delay = script?.delayMsBetweenEvents

    const cancelled = { value: false }
    const eventIterable = makeEventIterable(events, delay, cancelled)

    return {
      requestId,
      events: eventIterable,
      result: script?.resultError
        ? Promise.reject(script.resultError)
        : Promise.resolve(result),
      cancel: (cancelInput) => {
        recorded.cancelCalls.push({ reason: cancelInput?.reason })
        cancelled.value = true
        return Promise.resolve()
      },
      closeStream: (closeInput) => {
        recorded.closeStreamCalls.push({ reason: closeInput?.reason })
        cancelled.value = true
        return Promise.resolve()
      },
    }
  }

  async *runTurn(input: AcpRuntimeTurnInput): AsyncIterable<AcpRuntimeEvent> {
    const turn = this.startTurn(input)
    for await (const event of turn.events) yield event
    try {
      const result = await turn.result
      if (result.status === 'completed' || result.status === 'cancelled') {
        yield { type: 'done', stopReason: result.stopReason }
      } else {
        yield {
          type: 'error',
          message: result.error.message,
          code: result.error.code,
          retryable: result.error.retryable,
        }
      }
    } catch (err) {
      yield {
        type: 'error',
        message: err instanceof Error ? err.message : String(err),
      }
    }
  }

  cancel(input: { handle: AcpRuntimeHandle; reason?: string }): Promise<void> {
    this.cancelCalls.push(input)
    return Promise.resolve()
  }

  close(input: {
    handle: AcpRuntimeHandle
    reason: string
    discardPersistentState?: boolean
  }): Promise<void> {
    this.closeCalls.push(input)
    return Promise.resolve()
  }

  setMode(input: { handle: AcpRuntimeHandle; mode: string }): Promise<void> {
    this.setModeCalls.push(input)
    return Promise.resolve()
  }

  setConfigOption(input: {
    handle: AcpRuntimeHandle
    key: string
    value: string
  }): Promise<void> {
    this.setConfigOptionCalls.push(input)
    return Promise.resolve()
  }

  getStatus(input: { handle: AcpRuntimeHandle }): Promise<AcpRuntimeStatus> {
    this.getStatusCalls.push(input)
    return Promise.resolve(this.opts.status ?? DEFAULT_STATUS)
  }

  getCapabilities(
    input: { handle?: AcpRuntimeHandle } = {},
  ): AcpRuntimeCapabilities {
    this.getCapabilitiesCalls.push(input)
    return this.opts.capabilities ?? DEFAULT_CAPABILITIES
  }

  doctor(): Promise<AcpRuntimeDoctorReport> {
    this.doctorCalls.push(Date.now())
    return Promise.resolve(this.opts.doctorReport ?? DEFAULT_DOCTOR)
  }
}

function defaultHandle(input: AcpRuntimeEnsureInput): AcpRuntimeHandle {
  return {
    sessionKey: input.sessionKey,
    backend: 'mock-acpx',
    runtimeSessionName: `${input.sessionKey}::${input.agent}`,
    cwd: input.cwd,
    acpxRecordId: `mock-${input.sessionKey}`,
    backendSessionId:
      input.resumeSessionId ?? `mock-session-${input.sessionKey}`,
    agentSessionId: undefined,
  }
}

function makeEventIterable(
  events: AcpRuntimeEvent[],
  delayMs: number | undefined,
  cancelled: { value: boolean },
): AsyncIterable<AcpRuntimeEvent> {
  return {
    async *[Symbol.asyncIterator]() {
      for (const event of events) {
        if (cancelled.value) return
        if (delayMs && delayMs > 0) {
          await new Promise((resolve) => setTimeout(resolve, delayMs))
        }
        if (cancelled.value) return
        yield event
      }
    },
  }
}
