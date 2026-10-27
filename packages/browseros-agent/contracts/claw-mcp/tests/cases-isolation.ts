/**
 * What a page can see of BrowserOS's own scripts. The fixture hooks the built-ins a page-world
 * scan would call and records every DOM mutation, the way an anti-bot script would. `snapshot`,
 * `grep` and `evaluate` must leave that record empty; only `evaluate_in_page`, the explicit
 * main-world tool, runs where the page can watch.
 */

import type { CaseContext, ContractCase } from './cases'
import { expectOk } from './helpers'

const UNTRUSTED = /\[(END_)?UNTRUSTED_PAGE_CONTENT[^\]]*\]/g
const NUDGE = /Tip: this session is[\s\S]*$/

interface ProbeReport {
  calls: string[]
  mutations: string[]
}

/**
 * The value a script returned. Results arrive as a preface line, then the value fenced as
 * untrusted page content, then possibly the session rename nudge.
 */
function scriptValue(text: string): string {
  const lines = text
    .replace(UNTRUSTED, '')
    .replace(NUDGE, '')
    .trim()
    .split('\n')
  return lines[lines.length - 1]?.trim() ?? ''
}

async function script(
  ctx: CaseContext,
  tool: 'evaluate' | 'evaluate_in_page',
  page: number,
  code: string,
): Promise<string> {
  return scriptValue(
    expectOk(await ctx.mcp.callTool(tool, { page, code }), `${tool} ${code}`),
  )
}

async function resetProbe(ctx: CaseContext, page: number): Promise<void> {
  await script(ctx, 'evaluate_in_page', page, 'return isolationProbe.reset()')
}

async function probeReport(
  ctx: CaseContext,
  page: number,
): Promise<ProbeReport> {
  const text = await script(
    ctx,
    'evaluate_in_page',
    page,
    'return isolationProbe.report()',
  )
  return JSON.parse(text.slice(text.indexOf('{'), text.lastIndexOf('}') + 1))
}

function requireUnseen(report: ProbeReport, context: string): void {
  if (report.calls.length > 0 || report.mutations.length > 0) {
    throw new Error(
      `${context} was visible to the page: ${JSON.stringify(report)}`,
    )
  }
}

function requireRef(snapshot: string, label: string): void {
  const line = snapshot.split('\n').find((entry) => entry.includes(label))
  if (!line?.includes('[ref=')) {
    throw new Error(
      `snapshot gave no ref to "${label}": ${line ?? snapshot.slice(0, 600)}`,
    )
  }
}

export const isolationCases: ContractCase[] = [
  {
    name: 'isolation: snapshot and grep run no page script and write nothing',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/script-isolation.html'))
      await resetProbe(ctx, page)

      const snapshot = expectOk(
        await ctx.mcp.callTool('snapshot', { page }),
        'isolation snapshot',
      )
      expectOk(
        await ctx.mcp.callTool('grep', { page, pattern: 'control' }),
        'isolation grep',
      )

      requireUnseen(await probeReport(ctx, page), 'snapshot and grep')
      // Found without reading the page's JavaScript: a script-assigned onclick, a listener,
      // and an element inside a closed shadow root.
      requireRef(snapshot, 'Assigned onclick control')
      requireRef(snapshot, 'Listener control')
      requireRef(snapshot, 'Shadow control')
    },
  },
  {
    name: 'isolation: evaluate sees the DOM, not the page, and the page cannot see it',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/script-isolation.html'))
      await resetProbe(ctx, page)

      const seen = await script(
        ctx,
        'evaluate',
        page,
        'return JSON.stringify([typeof window.pageSecret, document.querySelectorAll("div").length, document.getElementById("result").textContent])',
      )
      if (!seen.includes('["undefined",2,"untouched"]')) {
        throw new Error(`evaluate saw the wrong world: ${seen}`)
      }
      const first = await script(
        ctx,
        'evaluate',
        page,
        'window.agentLedger = (window.agentLedger || 0) + 1; return window.agentLedger',
      )
      const second = await script(
        ctx,
        'evaluate',
        page,
        'window.agentLedger = (window.agentLedger || 0) + 1; return window.agentLedger',
      )
      if (first !== '1' || second !== '2') {
        throw new Error(`evaluate globals did not persist: ${first}, ${second}`)
      }
      requireUnseen(await probeReport(ctx, page), 'evaluate')

      // The same probe does catch a main-world script, so its silence above is meaningful.
      const inPage = await script(
        ctx,
        'evaluate_in_page',
        page,
        'return JSON.stringify([window.pageSecret, typeof window.agentLedger, document.querySelectorAll("div").length])',
      )
      if (!inPage.includes('["main-world-only","undefined",2]')) {
        throw new Error(`evaluate_in_page saw the wrong world: ${inPage}`)
      }
      const report = await probeReport(ctx, page)
      if (!report.calls.includes('querySelectorAll')) {
        throw new Error(
          `the page did not see evaluate_in_page: ${JSON.stringify(report)}`,
        )
      }
    },
  },
  {
    name: 'isolation: run sends page.evaluate and page.evaluateInPage to their worlds',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/script-isolation.html'))
      const result = expectOk(
        await ctx.mcp.callTool('run', {
          code: `const page = browser.page(${page});
            const isolated = await page.evaluate(() => typeof window.pageSecret);
            const inPage = await page.evaluateInPage(() => window.pageSecret);
            return 'isolated=' + isolated.value + ';page=' + inPage.value;`,
        }),
        'run worlds',
      )
      if (!result.includes('isolated=undefined;page=main-world-only')) {
        throw new Error(`run routed a script to the wrong world: ${result}`)
      }
    },
  },
]
