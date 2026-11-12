import { McpSession } from '../../../../../contracts/claw-mcp/tests/mcp-client'
import {
  type CaptchaFamily,
  type CaptchaPage,
  pageCheck,
  type Verdict,
} from '../fixtures/catalog'
import { clawServerPage } from './claw-server-page'
import type { PageSession, Timeouts } from './page-session'
import type { Server } from './preflight'
import { sdkPage } from './sdk-page'
import { notSolved, type SolveResult } from './solve-result'

export interface RunOptions {
  mcpUrl: string
  server: Server
  fixtureOrigin: string
  pages: readonly CaptchaPage[]
  tokenLengthExpression: (family: CaptchaFamily) => string
  paid?: boolean
  expectToolMissing?: boolean
  solveTimeout?: number
  probeTimeoutMs?: number
  navigationTimeoutMs?: number
  pollIntervalMs?: number
  /** Called as each page finishes, for progress on long runs. */
  onRecord?: (record: ResultRecord) => void
}
export interface ResultRecord extends SolveResult {
  id: string
  kind: string
  check: ReturnType<typeof pageCheck>
  url: string
  family: string
  expected: string[]
  latencyMs: number
  tokenLength: number | null
  callbackFired: boolean | null
  /** The page's own server, after the runner submitted its form. */
  site: 'accepted' | 'rejected' | 'n/a' | 'error'
  outcome: 'PASS' | 'FAIL' | 'UNVERIFIED'
  note: string
}
type Classification = { outcome: ResultRecord['outcome']; note: string }

/** How a solver that is not there answers: the SDK's unlisted method, or claw-server's missing tool. */
const ABSENT = new Set(['captcha_unavailable', 'tool_not_found'])
const errorCode = (solve: Pick<SolveResult, 'error'>) =>
  solve.error?.split('/').pop() ?? ''

type Solve = Pick<
  SolveResult,
  'verdict' | 'type' | 'reason' | 'paidTasks' | 'error'
>
const pass = (note = ''): Classification => ({ outcome: 'PASS', note })
const fail = (note = ''): Classification => ({ outcome: 'FAIL', note })

/** Why a solve cannot pass whatever its verdict: no call, a failed call, the wrong type or spend. */
function precondition(page: CaptchaPage, solve: Solve): string | null {
  if (solve.verdict === 'n/a') return ''
  if (solve.error) return `solve failed: ${solve.error}`
  if (page.expectType !== undefined && solve.type !== page.expectType)
    return `expected type ${page.expectType}, received ${solve.type ?? 'none'}`
  if (page.maxPaidTasks === undefined) return null
  if (solve.paidTasks == null) return 'paid task count missing'
  return solve.paidTasks > page.maxPaidTasks
    ? `paid tasks ${solve.paidTasks} exceed cap ${page.maxPaidTasks}`
    : null
}

/** An expected verdict, checked against the expected reason and the site's own answer. */
function expected(
  page: CaptchaPage,
  verdict: Verdict,
  reason: string,
  site: string,
): Classification {
  if (page.expectReason && !page.expectReason.includes(reason))
    return fail(
      `expected reason ${page.expectReason.join(' or ')}, received ${reason}`,
    )
  const needsSite =
    verdict === 'solved' ||
    (verdict === 'already_clear' && page.family !== 'none')
  if (needsSite && site === 'rejected') return fail('false positive')
  if (needsSite && site === 'error') return fail('independent check failed')
  if (verdict === 'token_applied' && !page.site)
    return pass('token delivered; no site check, not independently solved')
  return pass(needsSite && site === 'n/a' ? 'no independent check' : '')
}

/**
 * The page's outcome from the solver's own answer and the site's. A verdict passes only if the
 * page expects it, and a success on a CAPTCHA page only if the site accepted it too.
 */
export function classifyOutcome(
  page: CaptchaPage,
  solve: Solve,
  site: string,
  options: { paid?: boolean; expectToolMissing?: boolean } = {},
): Classification {
  if (options.expectToolMissing) {
    if (ABSENT.has(errorCode(solve))) return pass('solver absent, as expected')
    return fail(
      solve.verdict === 'n/a'
        ? ''
        : `the solver answered ${solve.error ?? solve.verdict}, but --expect-tool-missing was set`,
    )
  }
  const blocked = precondition(page, solve)
  if (blocked !== null) return fail(blocked)
  const verdict = solve.verdict as Verdict
  // Unsolved is never a guess, so a site that accepts the page proves it wrong, expected or
  // not: an unpaid demo must not turn that into UNVERIFIED.
  if (verdict === 'unsolved' && site === 'accepted')
    return fail('the site accepted a page the solver called unsolved')
  // The SDK reports claw-server's `unsupported` as `unsolved`; only the reason tells them apart.
  const refusalAllowed =
    page.allowUnsupported || page.expectReason?.includes('unsupported_type')
  if (
    verdict === 'unsolved' &&
    solve.reason === 'unsupported_type' &&
    !refusalAllowed
  )
    return fail('Neo refused this CAPTCHA type as unsupported')
  if (verdict === 'token_applied' && page.expect.includes('solved')) {
    if (site === 'accepted')
      return pass('token applied; site accepted on submit')
    return fail(
      site === 'rejected' ? 'false positive' : 'independent check failed',
    )
  }
  if (page.expect.includes(verdict))
    return expected(page, verdict, solve.reason, site)
  const unproven = verdict === 'unsolved' || verdict === 'unknown'
  if (page.mayNeedPaid && !options.paid && unproven)
    return {
      outcome: 'UNVERIFIED',
      note: `paid path unverified: ${solve.reason}`,
    }
  return fail()
}

function describe(error: unknown) {
  const message = error instanceof Error ? error.message : String(error)
  return message.length > 200 ? `${message.slice(0, 200)}…` : message
}

async function runPage(
  entry: CaptchaPage,
  page: PageSession,
  options: RunOptions,
): Promise<ResultRecord> {
  const url =
    entry.url ?? new URL(entry.path ?? '/', options.fixtureOrigin).href
  const record: ResultRecord = {
    ...notSolved(),
    id: entry.id,
    kind: entry.kind,
    check: pageCheck(entry),
    url,
    family: entry.family,
    expected: [...entry.expect],
    latencyMs: 0,
    tokenLength: null,
    callbackFired: null,
    site: entry.site ? 'error' : 'n/a',
    outcome: 'FAIL',
    note: '',
  }
  try {
    await page.open(url)
    // An absent solver refuses whatever the page shows, so absence runs skip the widget wait.
    if (!options.expectToolMissing && !(await page.ready(entry.ready)))
      record.note = 'widget never rendered'
    else {
      const started = performance.now()
      Object.assign(record, await page.solve())
      record.latencyMs = Math.round(performance.now() - started)
    }
    try {
      const state = (await page.observe(
        options.tokenLengthExpression(entry.family),
      )) as {
        tokenLength?: unknown
        callbackFired?: unknown
        fixtureTokenLength?: unknown
      } | null
      const length = state?.tokenLength ?? state?.fixtureTokenLength
      record.tokenLength =
        typeof length === 'number' && Number.isFinite(length) && length >= 0
          ? length
          : null
      record.callbackFired =
        typeof state?.callbackFired === 'boolean' ? state.callbackFired : null
    } catch {
      record.note ||= 'token observation failed'
    }
    if (entry.site) {
      try {
        const found = await page.submit(entry.site)
        if (found) record.site = found
        else record.note ||= 'site verdict timed out'
      } catch {
        record.note ||= 'site check failed'
      }
    }
  } catch (error) {
    record.note = `page run failed: ${describe(error)}`
  } finally {
    await page.close().catch(() => {})
  }
  const classification = classifyOutcome(entry, record, record.site, options)
  record.outcome = classification.outcome
  if (classification.note)
    record.note = [classification.note, record.note].filter(Boolean).join('; ')
  return record
}

/** Runs each page in its own tab, one after another, against a server preflight has checked. */
export async function runCaptchaE2E(
  options: RunOptions,
): Promise<ResultRecord[]> {
  const mcp = await McpSession.connect(
    options.mcpUrl.replace(/\/mcp\/?$/, ''),
    'captcha-e2e',
  )
  const timeouts: Timeouts = {
    probeMs: options.probeTimeoutMs ?? 20_000,
    navigationMs: options.navigationTimeoutMs ?? 30_000,
    solveMs: options.solveTimeout ?? 150_000,
    pollMs: options.pollIntervalMs ?? 200,
  }
  const records: ResultRecord[] = []
  try {
    for (const entry of options.pages) {
      const record = await runPage(
        entry,
        options.server === 'sdk'
          ? sdkPage(mcp, entry.id, timeouts)
          : clawServerPage(mcp, timeouts),
        options,
      )
      records.push(record)
      options.onRecord?.(record)
    }
    return records
  } finally {
    await mcp.close().catch(() => {})
  }
}

/** One line per finished page: the solver's exact answer, the site's, and the outcome. */
export function progressLine(r: ResultRecord): string {
  const verdict =
    r.status && r.status !== r.verdict
      ? `${r.verdict} (${r.status})`
      : r.verdict
  return `[captcha-e2e] ${r.id}: ${verdict} ${r.error ?? r.reason} type=${r.type ?? 'n/a'} paid=${r.paidTasks ?? 'n/a'} site=${r.site} -> ${r.outcome}${r.note ? ` (${r.note})` : ''}`
}

export const exitCode = (records: readonly ResultRecord[]) =>
  records.some((r) => r.outcome === 'FAIL') ? 1 : 0

/** Counts that keep site-accepted submissions apart from detections and paid work. */
export function summarize(records: readonly ResultRecord[]) {
  const passed = records.filter((r) => r.outcome === 'PASS')
  // A pass on an absent solver proves the absence only: no solve, delivery or detection.
  const answered = passed.filter((r) => !r.error)
  return {
    pass: passed.length,
    fail: records.filter((r) => r.outcome === 'FAIL').length,
    unverified: records.filter((r) => r.outcome === 'UNVERIFIED').length,
    siteAccepted: answered.filter(
      (r) => r.check === 'solve' && r.site === 'accepted',
    ).length,
    deliveredOnly: answered.filter((r) => r.check === 'deliver').length,
    detected: answered.filter((r) => r.check === 'detect').length,
    absent: passed.length - answered.length,
    paidTasks: records.reduce((sum, r) => sum + (r.paidTasks ?? 0), 0),
    // A solve that failed may still have paid before it stopped; only an absent solver did not.
    paidTasksUnreported: records.filter(
      (r) =>
        r.verdict !== 'n/a' && r.paidTasks == null && !ABSENT.has(errorCode(r)),
    ).length,
  }
}

export function renderSummary(records: readonly ResultRecord[]): string {
  const s = summarize(records)
  return [
    `PASS=${s.pass} FAIL=${s.fail} UNVERIFIED=${s.unverified}`,
    `site-accepted submissions: ${s.siteAccepted}`,
    `token deliveries without a site check: ${s.deliveredOnly}`,
    `detections and refusals: ${s.detected}`,
    ...(s.absent ? [`solver absent as expected: ${s.absent}`] : []),
    `paid tasks: ${s.paidTasks}${s.paidTasksUnreported ? ` (+${s.paidTasksUnreported} results without a count)` : ''}`,
  ].join(' · ')
}

export function renderTable(records: readonly ResultRecord[]): string {
  const cell = (value: unknown) =>
    String(value)
      .replaceAll('|', '\\|')
      .replace(/[\r\n]+/g, ' ')
  return [
    '| Page | Check | Expected | Verdict | Type | Reason | Paid | Site | Latency | Outcome | Note |',
    '| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |',
    ...records.map(
      (r) =>
        `| ${[
          r.id,
          r.check,
          r.expected.join(', '),
          r.status && r.status !== r.verdict
            ? `${r.verdict} (${r.status})`
            : r.verdict,
          r.type ?? 'n/a',
          r.error ?? r.reason,
          r.paidTasks ?? 'n/a',
          r.site,
          `${r.latencyMs} ms`,
          r.outcome,
          r.note,
        ]
          .map(cell)
          .join(' | ')} |`,
    ),
  ].join('\n')
}
