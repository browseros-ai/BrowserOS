import { parsePageId } from '../../claw-mcp/tests/helpers'
import {
  McpRequestError,
  McpSession,
  textOf,
} from '../../claw-mcp/tests/mcp-client'
import type { CaptchaFamily, CaptchaPage } from '../fixtures/catalog'
import { bodyProbe, observationProbe, readyProbe, submitProbe } from './probes'
import { parseSolveResult } from './solve-result'

export interface RunOptions {
  mcpUrl: string
  fixtureOrigin: string
  pages: readonly CaptchaPage[]
  tokenLengthExpression: (family: CaptchaFamily) => string
  paid?: boolean
  expectToolMissing?: boolean
  solveTimeout?: number
  probeTimeoutMs?: number
  pollIntervalMs?: number
}
export interface ResultRecord extends ReturnType<typeof parseSolveResult> {
  id: string
  kind: string
  url: string
  family: string
  expected: string[]
  latencyMs: number
  tokenLength: number | null
  callbackFired: boolean | null
  verdict: 'accepted' | 'rejected' | 'n/a' | 'error'
  outcome: 'PASS' | 'FAIL' | 'UNVERIFIED'
  note: string
}

function classifyAppliedToken(
  verdict: string,
): Pick<ResultRecord, 'outcome' | 'note'> {
  if (verdict === 'accepted')
    return { outcome: 'PASS', note: 'token applied; site accepted on submit' }
  return {
    outcome: 'FAIL',
    note:
      verdict === 'rejected' ? 'false positive' : 'independent check failed',
  }
}

export function classifyOutcome(
  page: CaptchaPage,
  status: string,
  verdict: string,
  reason: string,
  options: { paid?: boolean; expectToolMissing?: boolean } = {},
): { outcome: ResultRecord['outcome']; note: string } {
  if (status === 'tool_not_found' || options.expectToolMissing)
    return {
      outcome:
        status === 'tool_not_found' && options.expectToolMissing
          ? 'PASS'
          : 'FAIL',
      note: '',
    }
  if (status === 'token_applied' && page.expect.includes('solved'))
    return classifyAppliedToken(verdict)
  if (page.expect.some((expected) => expected === status)) {
    const needsVerdict =
      status === 'solved' ||
      (status === 'already_clear' && page.family !== 'none')
    if (needsVerdict && verdict === 'rejected')
      return { outcome: 'FAIL', note: 'false positive' }
    if (needsVerdict && verdict === 'error')
      return { outcome: 'FAIL', note: 'independent check failed' }
    return {
      outcome: 'PASS',
      note: needsVerdict && verdict === 'n/a' ? 'no independent check' : '',
    }
  }
  if (
    page.mayNeedPaid &&
    !options.paid &&
    ['failed', 'needs_human'].includes(status)
  )
    return { outcome: 'UNVERIFIED', note: `paid path unverified: ${reason}` }
  return { outcome: 'FAIL', note: '' }
}

function payload(text: string): unknown {
  // The opening line contains a trust notice; readiness notes follow the end fence.
  const framed = text.match(
    /^\[UNTRUSTED_PAGE_CONTENT[^\]]*\][^\n]*\n([\s\S]*?)\n\[END_UNTRUSTED_PAGE_CONTENT[^\]]*\]/,
  )
  const clean = (framed?.[1] ?? text)
    .replace(/Tip: this session is[\s\S]*$/, '')
    .trim()
  try {
    return JSON.parse(clean)
  } catch {
    return clean
  }
}

export async function runCaptchaE2E(
  options: RunOptions,
): Promise<ResultRecord[]> {
  const mcp = await McpSession.connect(
    options.mcpUrl.replace(/\/mcp\/?$/, ''),
    'captcha-e2e',
  )
  const records: ResultRecord[] = []
  const timeout = options.probeTimeoutMs ?? 20_000
  try {
    const hasTool = (await mcp.listTools()).some(
      (tool) => tool.name === 'solve_captcha',
    )
    for (const entry of options.pages) {
      const url =
        entry.url ?? new URL(entry.path ?? '/', options.fixtureOrigin).href
      const record: ResultRecord = {
        ...parseSolveResult({ content: [] }),
        id: entry.id,
        kind: entry.kind,
        url,
        family: entry.family,
        expected: [...entry.expect],
        latencyMs: 0,
        tokenLength: null,
        callbackFired: null,
        verdict: entry.verdict ? 'error' : 'n/a',
        outcome: 'FAIL',
        note: '',
      }
      let page: number | undefined
      const evaluate = async (code: string) => {
        const result = await mcp.callTool('evaluate', { page, code }, timeout)
        if (result.isError) throw new Error('page probe failed')
        return payload(textOf(result))
      }
      const poll = async (probe: () => Promise<boolean>) => {
        const deadline = Date.now() + timeout
        do {
          try {
            if (await probe()) return true
          } catch {}
          await Bun.sleep(options.pollIntervalMs ?? 200)
        } while (Date.now() < deadline)
        return false
      }
      try {
        page = parsePageId(await mcp.callTool('tabs', { action: 'new', url }))
        if (!hasTool) record.status = 'tool_not_found'
        else if (
          !(await poll(
            async () => (await evaluate(readyProbe(entry.ready))) === true,
          ))
        )
          record.note = 'widget never rendered'
        else {
          const started = performance.now()
          try {
            Object.assign(
              record,
              parseSolveResult(
                await mcp.callTool(
                  'solve_captcha',
                  { page },
                  options.solveTimeout ?? 150_000,
                ),
              ),
            )
          } catch (error) {
            record.status =
              error instanceof McpRequestError && error.code === -32601
                ? 'tool_not_found'
                : 'error'
            record.note =
              record.status === 'error' ? 'solve request failed' : ''
          } finally {
            record.latencyMs = Math.round(performance.now() - started)
          }
        }
        try {
          const state = (await evaluate(
            observationProbe(options.tokenLengthExpression(entry.family)),
          )) as {
            tokenLength?: unknown
            callbackFired?: unknown
            fixtureTokenLength?: unknown
          }
          const length = state.tokenLength ?? state.fixtureTokenLength
          record.tokenLength =
            typeof length === 'number' && Number.isFinite(length) && length >= 0
              ? length
              : null
          record.callbackFired =
            typeof state.callbackFired === 'boolean'
              ? state.callbackFired
              : null
        } catch {
          record.note ||= 'token observation failed'
        }
        if (entry.verdict) {
          const verdict = entry.verdict
          try {
            await evaluate(submitProbe(verdict.submitSelector))
            const confirmed = await poll(async () => {
              const found = await evaluate(bodyProbe(verdict))
              if (found === 'accepted' || found === 'rejected')
                record.verdict = found
              return record.verdict !== 'error'
            })
            if (!confirmed) record.note ||= 'verdict timed out'
          } catch {
            record.note ||= 'verdict check failed'
          }
        }
      } catch {
        record.note = 'page run failed'
      } finally {
        if (page !== undefined)
          await mcp.callTool('tabs', { action: 'close', page }).catch(() => {})
      }
      const classification = classifyOutcome(
        entry,
        record.status,
        record.verdict,
        record.reason,
        options,
      )
      record.outcome = classification.outcome
      if (classification.note)
        record.note = [classification.note, record.note]
          .filter(Boolean)
          .join('; ')
      records.push(record)
    }
    return records
  } finally {
    await mcp.close().catch(() => {})
  }
}

export const exitCode = (records: readonly ResultRecord[]) =>
  records.some((r) => r.outcome === 'FAIL') ? 1 : 0
export function renderTable(records: readonly ResultRecord[]): string {
  const cell = (value: unknown) =>
    String(value)
      .replaceAll('|', '\\|')
      .replace(/[\r\n]+/g, ' ')
  return [
    '| Page | Expected | Result | Type | Path | Verdict | Latency | Cost | Outcome | Note |',
    '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |',
    ...records.map(
      (r) =>
        `| ${[
          r.id,
          r.expected.join(', '),
          r.status,
          r.type,
          r.method,
          r.verdict,
          `${r.latencyMs} ms`,
          r.costUsd ?? 'n/a',
          r.outcome,
          [
            r.status === 'token_applied' && r.reason !== 'n/a' ? r.reason : '',
            r.note,
          ]
            .filter(Boolean)
            .join('; '),
        ]
          .map(cell)
          .join(' | ')} |`,
    ),
  ].join('\n')
}
