import { randomUUID } from 'node:crypto'
import { checkHost } from './host-checks'
import { canonicalJson, jsonObject } from './json'
import type {
  CaseResult,
  CaseSpec,
  HostClient,
  JsonObject,
  JsonValue,
  RunReply,
  RunRequest,
  SettingsPatch,
} from './model'
import type { BorrowedTargets } from './root-cdp'
import { assertExpectedFailure } from './strict-xfail'

interface StepEvidence {
  scripts: readonly string[]
  replies: readonly JsonValue[]
}

export interface CaseEvidence {
  id: string
  taskId: string
  steps: readonly StepEvidence[]
  cleanup?: JsonValue
  result: CaseResult
}

interface HarnessOptions {
  run(request: RunRequest): Promise<RunReply>
  host: HostClient
  targets: BorrowedTargets
  base: string
  base2: string
  scale: 1 | 2
  common: string
  loadBody(script: string): Promise<string>
  recordEvidence?(evidence: CaseEvidence): Promise<void>
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function patchMatches(settings: JsonObject, patch: SettingsPatch): boolean {
  if ('tabs' in patch) {
    return (
      jsonObject(settings.tabs, 'Effective tab settings').scope ===
      patch.tabs.scope
    )
  }
  return (
    jsonObject(settings.trace, 'Effective trace settings').valueBytes ===
    patch.trace.valueBytes
  )
}

async function executeSteps(options: {
  spec: CaseSpec
  script(path: string): Promise<RunReply>
  replies: RunReply[]
  evidence: StepEvidence[]
  prior: JsonValue[]
}): Promise<void> {
  for (const step of options.spec.steps) {
    if (step.kind === 'script') {
      const reply = await options.script(step.script)
      options.replies.push(reply)
      options.evidence.push({ scripts: [step.script], replies: [reply.raw] })
      options.prior.push(reply.kind === 'ok' ? reply.value : null)
      if (reply.kind === 'error') break
    } else {
      // allSettled joins every independent request before cleanup can start. Promise.all
      // alone would permit a rejected transport to race cleanup against its sibling runs.
      const settled = await Promise.allSettled(step.scripts.map(options.script))
      const group: RunReply[] = []
      const errors: string[] = []
      for (const result of settled) {
        if (result.status === 'fulfilled') group.push(result.value)
        else errors.push(errorMessage(result.reason))
      }
      options.replies.push(...group)
      options.evidence.push({
        scripts: step.scripts,
        replies: group.map((reply) => reply.raw),
      })
      if (errors.length)
        throw new Error(`Parallel MCP transport failed: ${errors.join('; ')}`)
      options.prior.push(
        group.map((reply) => (reply.kind === 'ok' ? reply.value : null)),
      )
      if (group.some((reply) => reply.kind === 'error')) break
    }
  }
}

async function cleanupCase(options: {
  borrowed?: string
  taskId: string
  savedSettings?: JsonObject
  targets: BorrowedTargets
  host: HostClient
  run(code: string, timeoutMs: number): Promise<RunReply>
}): Promise<{ errors: string[]; reply?: JsonValue }> {
  const errors: string[] = []
  let reply: JsonValue | undefined
  if (options.borrowed) {
    try {
      await options.targets.close(options.borrowed)
    } catch (error) {
      errors.push(`Borrowed target: ${errorMessage(error)}`)
    }
  }
  try {
    // Closing this named task cannot sweep tabs owned by another task, even while the
    // borrowed-page contract has temporarily enabled browser-wide visibility.
    const result = await options.run(
      `await (await neo.task(${JSON.stringify(options.taskId)})).close(); return true;`,
      20_000,
    )
    reply = result.raw
    if (result.kind === 'error' || result.value !== true)
      throw new Error(`Task cleanup failed: ${result.text}`)
  } catch (error) {
    errors.push(errorMessage(error))
  }
  if (options.savedSettings) {
    try {
      await options.host.restoreSettings(options.savedSettings)
      const restored = await options.host.settings()
      if (canonicalJson(options.savedSettings) !== canonicalJson(restored))
        throw new Error('SDK settings were not restored exactly')
    } catch (error) {
      errors.push(`Settings restoration: ${errorMessage(error)}`)
    }
  }
  return { errors, reply }
}

/**
 * One check owns setup, all ordered runs, host assertions and every cleanup attempt. Settings
 * and profile state are shared across checks, so uncertain runs and failed cleanup poison reuse.
 */
export class Harness {
  #poisoned = false

  constructor(private readonly options: HarnessOptions) {}

  get poisoned(): boolean {
    return this.#poisoned
  }

  async check(spec: CaseSpec): Promise<CaseResult> {
    if (this.#poisoned)
      throw new Error(
        'Uncertain case state prevents further browser state reuse',
      )
    const started = performance.now()
    const taskId = `desktop-${spec.id}-${randomUUID()}`
    const replies: RunReply[] = []
    const evidence: StepEvidence[] = []
    // Each MCP call is a fresh script. Sequential steps receive earlier returned values;
    // a parallel step appends an ordered array after all four replies settle.
    const prior: JsonValue[] = []
    let savedSettings: JsonObject | undefined
    let borrowed: string | undefined
    let failure: string | undefined
    let outcome: { status: 'pass' } | { status: 'xfail'; reason: string } = {
      status: 'pass',
    }

    const run = async (
      code: string,
      timeoutMs: number,
      driverTimeoutMs?: number,
    ): Promise<RunReply> => {
      try {
        return await this.options.run({
          code,
          timeoutMs,
          ...(driverTimeoutMs === undefined ? {} : { driverTimeoutMs }),
          taskId,
          requestId: `desktop-${randomUUID()}`,
        })
      } catch (error) {
        // A missing or undecodable reply cannot prove the submitted run stopped. Cleanup is
        // still attempted, then the invocation must end so an unknown run cannot touch a later case.
        this.#poisoned = true
        throw error
      }
    }
    const script = async (path: string): Promise<RunReply> => {
      const args = JSON.stringify({
        base: this.options.base,
        base2: this.options.base2,
        prior,
      })
      const body = await this.options.loadBody(path)
      // The acceptance probe's driver patience is independent of the SDK budget sent on
      // the wire. Cleanup calls below omit that override and retain their ordinary budget.
      return await run(
        `const args = ${args};\n${this.options.common}\n${body}`,
        spec.timeoutMs,
        spec.driverTimeoutMs,
      )
    }

    try {
      if (spec.settings) {
        savedSettings = await this.options.host.settings()
        await this.options.host.patchSettings(spec.settings)
        if (!patchMatches(await this.options.host.settings(), spec.settings)) {
          throw new Error('Scoped SDK settings patch did not take effect')
        }
      }
      if (spec.hostSetup?.kind === 'borrowed-page') {
        const url = spec.hostSetup.urlTemplate
          .replace('{base}', this.options.base)
          .replace(
            '{encodeURIComponent(base2 + "/upload.html?parentPicker=1")}',
            encodeURIComponent(
              `${this.options.base2}/upload.html?parentPicker=1`,
            ),
          )
        try {
          borrowed = await this.options.targets.create(url)
        } catch (error) {
          // Root CDP may have created the target before its reply disappeared. Without its ID,
          // only the outer launcher's private browser shutdown proves the target is gone.
          this.#poisoned = true
          throw error
        }
      }
      if (spec.hostSetup?.kind === 'device-scale' && this.options.scale !== 2) {
        throw new Error(
          'The density contract requires a native device-scale-factor=2 browser launch',
        )
      }

      await executeSteps({ spec, script, replies, evidence, prior })

      const failed = replies.find((reply) => reply.kind === 'error')
      if (spec.expectation.kind === 'xfail') {
        const reply = failed ?? replies.at(-1)
        if (!reply)
          throw new Error('Expected failure case produced no MCP reply')
        assertExpectedFailure({ expectation: spec.expectation, reply })
        outcome = { status: 'xfail', reason: spec.expectation.reason }
      } else if (failed && spec.hostCheck?.kind !== 'run-error') {
        throw new Error(
          `${failed.error.owner}/${failed.error.code}: ${failed.error.message}`,
        )
      }
      if (spec.hostCheck) {
        await checkHost({
          spec: spec.hostCheck,
          replies,
          host: this.options.host,
        })
      }
    } catch (error) {
      failure = errorMessage(error)
    }

    const cleanupStarted = performance.now()
    const cleanup = await cleanupCase({
      borrowed,
      taskId,
      savedSettings,
      targets: this.options.targets,
      host: this.options.host,
      run,
    })
    if (cleanup.errors.length) {
      this.#poisoned = true
      failure = [failure, ...cleanup.errors].filter(Boolean).join('; ')
    }
    const base = {
      id: spec.id,
      file: spec.file,
      name: spec.name,
      durationMs: cleanupStarted - started,
      cleanupMs: performance.now() - cleanupStarted,
      runIds: replies.map((reply) => reply.runId),
    }
    const result: CaseResult = failure
      ? { ...base, status: 'fail', failure }
      : { ...base, ...outcome }
    try {
      await this.options.recordEvidence?.({
        id: spec.id,
        taskId,
        steps: evidence,
        cleanup: cleanup.reply,
        result,
      })
      return result
    } catch (error) {
      return {
        ...base,
        status: 'fail',
        failure: `Case evidence: ${errorMessage(error)}`,
      }
    }
  }
}
