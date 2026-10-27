/**
 * What a page can see of BrowserOS. The fixture hooks the built-ins a page-world scan would call
 * and records every DOM mutation, the way an anti-bot script would. Reading the page (`snapshot`,
 * `grep`) must leave that record empty; only `evaluate_in_page`, the explicit tool for running the
 * agent's own script, runs where the page can watch.
 */

import type { CaseContext, ContractCase } from './cases'
import { expectError, expectOk } from './helpers'

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

async function inPage(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<string> {
  return scriptValue(
    expectOk(
      await ctx.mcp.callTool('evaluate_in_page', { page, code }),
      `evaluate_in_page ${code}`,
    ),
  )
}

async function probeReport(
  ctx: CaseContext,
  page: number,
): Promise<ProbeReport> {
  const text = await inPage(ctx, page, 'return isolationProbe.report()')
  return JSON.parse(text.slice(text.indexOf('{'), text.lastIndexOf('}') + 1))
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
      await inPage(ctx, page, 'return isolationProbe.reset()')

      const snapshot = expectOk(
        await ctx.mcp.callTool('snapshot', { page }),
        'isolation snapshot',
      )
      expectOk(
        await ctx.mcp.callTool('grep', { page, pattern: 'control' }),
        'isolation grep',
      )

      const unseen = await probeReport(ctx, page)
      if (unseen.calls.length > 0 || unseen.mutations.length > 0) {
        throw new Error(
          `snapshot and grep were visible to the page: ${JSON.stringify(unseen)}`,
        )
      }
      // Found without reading the page's JavaScript: a script-assigned onclick, a listener,
      // and an element inside a closed shadow root.
      requireRef(snapshot, 'Assigned onclick control')
      requireRef(snapshot, 'Listener control')
      requireRef(snapshot, 'Shadow control')
      // A labelled child with its own handler inside a pointer-cursor card keeps its own ref.
      requireRef(snapshot, 'Card delete control')

      // The same probe does catch a script run in the page, so its silence above means something.
      const secret = await inPage(
        ctx,
        page,
        'return document.querySelectorAll("div").length + ":" + window.pageSecret',
      )
      if (secret !== '4:main-world-only') {
        throw new Error(`evaluate_in_page did not run in the page: ${secret}`)
      }
      const seen = await probeReport(ctx, page)
      if (!seen.calls.includes('querySelectorAll')) {
        throw new Error(
          `the page did not see evaluate_in_page: ${JSON.stringify(seen)}`,
        )
      }
    },
  },
  {
    name: 'isolation: a named control inside a clickable card is its own target',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/script-isolation.html'))
      const snapshot = expectOk(
        await ctx.mcp.callTool('snapshot', { page }),
        'card snapshot',
      )
      const line = snapshot
        .split('\n')
        .find((entry) => entry.includes('Card delete control'))
      const ref = line?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!ref) {
        throw new Error(
          `the card's own control got no ref: ${line ?? snapshot}`,
        )
      }
      expectOk(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref }),
        'click the card control',
      )
      const result = await inPage(
        ctx,
        page,
        'return document.getElementById("result").textContent',
      )
      if (result !== 'card delete clicked') {
        throw new Error(
          `clicking the control reached the wrong handler: ${result}`,
        )
      }
    },
  },
  {
    name: 'isolation: run scripts the page only through page.evaluateInPage',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/script-isolation.html'))
      const result = expectOk(
        await ctx.mcp.callTool('run', {
          code: `const seen = await browser.page(${page}).evaluateInPage(() => window.pageSecret);
            return 'page=' + seen.value;`,
        }),
        'run evaluateInPage',
      )
      if (!result.includes('page=main-world-only')) {
        throw new Error(`run did not reach the page world: ${result}`)
      }

      const removed = expectError(
        await ctx.mcp.callTool('run', {
          code: `return await browser.page(${page}).evaluate(() => 1);`,
        }),
        'run page.evaluate',
      )
      if (!removed.includes('evaluateInPage')) {
        throw new Error(
          `page.evaluate did not point at evaluateInPage: ${removed}`,
        )
      }
    },
  },
]
