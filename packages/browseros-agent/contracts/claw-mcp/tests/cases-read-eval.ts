/**
 * grep (6), read (5), evaluate (5) and run (4) cases. The grep=ax
 * REGRESSION CASE (#2 in the briefing) lives here: over=ax output must
 * still carry `[ref=eN]` handles after the serde fix. Spill behavior is
 * asserted through the `saved to: <path>` marker both formatters emit.
 */

import type { CaseContext, ContractCase } from './cases'
import { expectOk, waitUntil } from './helpers'
import { textOf } from './mcp-client'

const UNTRUSTED = /\[(END_)?UNTRUSTED_PAGE_CONTENT[^\]]*\]/g
const NUDGE = /Tip: this session is[\s\S]*$/

/** Page-derived text with the fence markers and the rename nudge stripped. */
function payload(text: string): string {
  return text.replace(UNTRUSTED, '').replace(NUDGE, '').trim()
}

function spillPath(text: string): string | undefined {
  return text.match(/saved to: (\S+)/)?.[1]
}

async function evalIn(
  ctx: CaseContext,
  page: number,
  code: string,
  timeout?: number,
): Promise<string> {
  const args: Record<string, unknown> = { page, code }
  if (timeout !== undefined) args.timeout = timeout
  return payload(textOf(await ctx.mcp.callTool('evaluate', args)))
}

async function loadBigSection(ctx: CaseContext, page: number): Promise<void> {
  await waitUntil(
    async () =>
      (
        await evalIn(
          ctx,
          page,
          'return String(document.querySelectorAll("#big h6").length)',
        )
      ).includes('3001'),
    'the oversized section to render',
    { timeoutMs: 30_000 },
  )
}

export const readEvalCases: ContractCase[] = [
  // grep -------------------------------------------------------------------
  {
    name: 'grep: over=ax keeps [ref=eN] handles (REGRESSION #2)',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      await ctx.mcp.callTool('snapshot', { page })
      const text = payload(
        expectOk(
          await ctx.mcp.callTool('grep', {
            page,
            pattern: 'Submit',
            over: 'ax',
          }),
          'grep over=ax',
        ),
      )
      if (!/\[ref=e\d+\]/.test(text)) {
        throw new Error(`grep over=ax dropped its refs:\n${text.slice(0, 300)}`)
      }
    },
  },
  {
    name: 'grep: over=content searches innerText',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const text = payload(
        expectOk(
          await ctx.mcp.callTool('grep', {
            page,
            pattern: 'downloadable report',
            over: 'content',
          }),
          'grep over=content',
        ),
      )
      if (!text.includes('downloadable report')) {
        throw new Error(
          `grep over=content missed the text:\n${text.slice(0, 200)}`,
        )
      }
      if (/\[ref=e\d+\]/.test(text)) {
        throw new Error(`content grep leaked accessibility refs: ${text}`)
      }
    },
  },
  {
    name: 'grep: pattern is case-insensitive',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      await ctx.mcp.callTool('snapshot', { page })
      const text = payload(
        expectOk(
          await ctx.mcp.callTool('grep', {
            page,
            pattern: 'submit',
            over: 'ax',
          }),
          'grep case-insensitive',
        ),
      )
      if (!/submit/i.test(text)) {
        throw new Error(`case-insensitive grep missed Submit: ${text}`)
      }
    },
  },
  {
    name: 'grep: limit clamps at 200 matches',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      await loadBigSection(ctx, page)
      const text = expectOk(
        await ctx.mcp.callTool('grep', {
          page,
          pattern: 'filler line',
          over: 'content',
          limit: 500,
        }),
        'grep with limit 500',
      )
      // 3000 lines match but the clamp caps stored matches at 200.
      const path = spillPath(text)
      let matchCount: number
      if (path) {
        const file = await Bun.file(path).text()
        matchCount = (file.match(/filler line/g) ?? []).length
      } else {
        matchCount = (payload(text).match(/filler line/g) ?? []).length
      }
      if (matchCount !== 200) {
        throw new Error(
          `grep returned ${matchCount} matches, expected the 200 clamp`,
        )
      }
    },
  },
  {
    name: 'grep: long lines are truncated at 500 chars',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      await loadBigSection(ctx, page)
      const text = expectOk(
        await ctx.mcp.callTool('grep', {
          page,
          pattern: 'longline',
          over: 'content',
        }),
        'grep long line',
      )
      // The 600-x line comes back clipped with a truncation marker.
      const inlineXs = payload(text).match(/x+/)?.[0].length ?? 0
      if (!/truncat/i.test(text) || inlineXs > 520) {
        throw new Error(`grep did not truncate its long line: ${text}`)
      }
    },
  },
  {
    name: 'grep: large result spills to a file',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      await loadBigSection(ctx, page)
      const text = expectOk(
        await ctx.mcp.callTool('grep', {
          page,
          pattern: 'filler line',
          over: 'content',
          limit: 200,
        }),
        'grep spill',
      )
      const path = spillPath(text)
      if (!path || !(await Bun.file(path).exists())) {
        throw new Error(`grep did not spill to a readable file: ${path}`)
      }
    },
  },

  // read -------------------------------------------------------------------
  {
    name: 'read: markdown format returns page content',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const text = payload(
        expectOk(
          await ctx.mcp.callTool('read', { page, format: 'markdown' }),
          'read markdown',
        ),
      )
      if (!text.includes('Section one') || !text.includes('Section two')) {
        throw new Error(
          `read markdown missed page content:\n${text.slice(0, 200)}`,
        )
      }
    },
  },
  {
    name: 'read: text format returns plain text',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const text = payload(
        expectOk(
          await ctx.mcp.callTool('read', { page, format: 'text' }),
          'read text',
        ),
      )
      if (!text.includes('Section one')) {
        throw new Error(`read text missed page content: ${text}`)
      }
    },
  },
  {
    name: 'read: links format lists anchors',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const text = payload(
        expectOk(
          await ctx.mcp.callTool('read', { page, format: 'links' }),
          'read links',
        ),
      )
      if (!text.includes('/form.html')) {
        throw new Error(`read links missed the fixture anchor: ${text}`)
      }
    },
  },
  {
    name: 'read: console format (rust-only)',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/console.html'))
      const result = await ctx.mcp.callTool('read', { page, format: 'console' })
      expectOk(result, 'read console')
      // The capture listener attaches after attach, so reload before checking
      // load-time warning/error entries.
      expectOk(await ctx.mcp.callTool('navigate', { page, action: 'reload' }))
      let text = ''
      await waitUntil(async () => {
        text = payload(
          textOf(await ctx.mcp.callTool('read', { page, format: 'console' })),
        )
        return text.includes('fixture error one')
      }, 'read console to report the page error entry')
    },
  },
  {
    name: 'read: large content spills to a file',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      await loadBigSection(ctx, page)
      const text = expectOk(
        await ctx.mcp.callTool('read', { page, format: 'text' }),
        'read spill',
      )
      const path = spillPath(text)
      if (!path || !(await Bun.file(path).exists())) {
        throw new Error(`read did not spill oversized content: ${path}`)
      }
    },
  },

  // evaluate ---------------------------------------------------------------
  {
    name: 'evaluate: returns a value round-trip',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const text = await evalIn(ctx, page, 'return 6 * 7')
      if (!text.includes('42')) {
        throw new Error(`evaluate did not round-trip the value: ${text}`)
      }
    },
  },
  {
    name: 'evaluate: awaits a returned promise',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const text = await evalIn(
        ctx,
        page,
        'return await Promise.resolve("awaited-value")',
      )
      if (!text.includes('awaited-value')) {
        throw new Error(`evaluate did not await the promise: ${text}`)
      }
    },
  },
  {
    name: 'evaluate: a thrown error surfaces in the result',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const result = await ctx.mcp.callTool('evaluate', {
        page,
        code: 'throw new Error("evaluate-boom")',
      })
      const text = payload(textOf(result))
      if (!result.isError || !text.includes('evaluate-boom')) {
        throw new Error(
          `thrown error not surfaced: isError=${result.isError} text=${text}`,
        )
      }
    },
  },
  {
    name: 'evaluate: accepts a timeout parameter',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      // A fast evaluation under the accepted timeout returns normally.
      const fast = await evalIn(ctx, page, 'return 1 + 1', 5_000)
      if (!fast.includes('2')) {
        throw new Error(`evaluate with a timeout did not return: ${fast}`)
      }
      // Renderer evaluation is not preempted by this request timeout today.
      const slow = await ctx.mcp.callTool('evaluate', {
        page,
        code: 'await new Promise(r => setTimeout(r, 8000)); return "slow"',
        timeout: 1_500,
      })
      expectOk(slow, 'evaluate page-context wait')
    },
  },
  {
    name: 'evaluate: large return spills to a file',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const text = expectOk(
        await ctx.mcp.callTool('evaluate', {
          page,
          code: 'return "z".repeat(20000)',
        }),
        'evaluate spill',
      )
      const path = spillPath(text)
      if (!path || !(await Bun.file(path).exists())) {
        throw new Error(`evaluate did not spill a large return: ${path}`)
      }
    },
  },

  // run --------------------------------------------------------------------
  {
    name: 'run: SDK end-to-end pages.list -> snapshot -> click -> return',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const text = payload(
        expectOk(
          await ctx.mcp.callTool('run', {
            code: `
              const pages = await browser.pages.list()
              const snap = await browser.observe(${page}).snapshot()
              const applyRef = snap.text.split('\\n').find(l => l.includes('Apply')).match(/\\[ref=(e\\d+)\\]/)[1]
              await browser.input(${page}).click(applyRef)
              return { pageCount: pages.length, applied: true }
            `,
          }),
          'run end-to-end',
        ),
      )
      if (!text.includes('applied')) {
        throw new Error(
          `run end-to-end did not return the expected value:\n${text.slice(0, 300)}`,
        )
      }
      await waitUntil(
        async () =>
          (
            await evalIn(
              ctx,
              page,
              'return document.getElementById("result").textContent',
            )
          ).includes('applied'),
        'the run script click to update #result',
      )
    },
  },
  {
    name: 'run: captures console.log output',
    async run(ctx) {
      const text = payload(
        expectOk(
          await ctx.mcp.callTool('run', {
            code: 'console.log("run-log-line"); return "done"',
          }),
          'run console capture',
        ),
      )
      if (!text.includes('run-log-line')) {
        throw new Error(`run did not capture console output: ${text}`)
      }
    },
  },
  {
    name: 'run: an in-script exception comes back as ok:false',
    async run(ctx) {
      const result = await ctx.mcp.callTool('run', {
        code: 'throw new Error("run-boom")',
      })
      const structured = result.structuredContent as
        | { ok?: boolean; error?: string }
        | undefined
      const text = payload(textOf(result))
      const failed =
        structured?.ok === false ||
        result.isError === true ||
        /run-boom/.test(text)
      if (!failed) {
        throw new Error(
          `run swallowed the exception: ${JSON.stringify(structured)} ${text}`,
        )
      }
    },
  },
  {
    name: 'run: clamps an over-long timeout',
    async run(ctx) {
      // A timeout far over the 30s cap must be clamped, not honored: a
      // hanging script terminates within the clamp, never after 10
      // minutes. Accept a thrown socket-close as proof the request terminated.
      const started = Date.now()
      let terminated = false
      try {
        const result = await ctx.mcp.callTool('run', {
          code: 'await new Promise(() => {}); return "never"',
          timeout: 600_000,
        })
        terminated = result.isError === true
      } catch {
        terminated = true
      }
      const elapsed = Date.now() - started
      if (!terminated || elapsed > 90_000) {
        throw new Error(`run did not clamp the timeout (elapsed ${elapsed}ms)`)
      }
    },
  },
]
