import { parsePageId } from '../../../../../contracts/claw-mcp/tests/helpers'
import {
  McpRequestError,
  type McpSession,
  textOf,
} from '../../../../../contracts/claw-mcp/tests/mcp-client'
import type { PageSession, Timeouts } from './page-session'
import { bodyProbe, observationProbe, readyProbe, submitProbe } from './probes'
import { notSolved, parseClawServerSolve } from './solve-result'

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

/**
 * A page over claw-server's tools (`tabs`, `evaluate_in_page`, `solve_captcha`), for the
 * production Neo image, which still runs claw-server. Only for an endpoint given with
 * `--server claw-server`; everything the testbed launches is the Neo SDK.
 */
export function clawServerPage(
  mcp: McpSession,
  timeouts: Timeouts,
): PageSession {
  let page: number | undefined
  const evaluate = async (code: string) => {
    const result = await mcp.callTool(
      'evaluate_in_page',
      { page, code },
      timeouts.probeMs,
    )
    if (result.isError) throw new Error('page probe failed')
    return payload(textOf(result))
  }
  const poll = async (probe: () => Promise<boolean>) => {
    const deadline = Date.now() + timeouts.probeMs
    do {
      try {
        if (await probe()) return true
      } catch {}
      await Bun.sleep(timeouts.pollMs)
    } while (Date.now() < deadline)
    return false
  }
  return {
    async open(url) {
      page = parsePageId(await mcp.callTool('tabs', { action: 'new', url }))
    },
    ready: (expression) =>
      poll(async () => (await evaluate(readyProbe(expression))) === true),
    async solve() {
      try {
        return parseClawServerSolve(
          await mcp.callTool('solve_captcha', { page }, timeouts.solveMs),
        )
      } catch (error) {
        const missing =
          error instanceof McpRequestError && error.code === -32601
        return {
          ...notSolved(),
          verdict: 'error',
          status: missing ? 'tool_not_found' : 'error',
          error: missing ? 'mcp/tool_not_found' : 'mcp/request_failed',
        }
      }
    },
    observe: (tokenExpression) => evaluate(observationProbe(tokenExpression)),
    async submit(site) {
      await evaluate(submitProbe(site.submitSelector))
      let found: 'accepted' | 'rejected' | null = null
      await poll(async () => {
        const value = await evaluate(bodyProbe(site))
        if (value === 'accepted' || value === 'rejected') found = value
        return found !== null
      })
      return found
    },
    async close() {
      if (page !== undefined)
        await mcp.callTool('tabs', { action: 'close', page })
    },
  }
}
