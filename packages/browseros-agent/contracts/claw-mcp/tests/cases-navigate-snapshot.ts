/**
 * navigate (5), snapshot (8) and diff (3) cases — the ref-minting
 * heart of the contract. The two REGRESSION CASES from the
 * backendDOMNodeId serde break live here: refs must mint on a real
 * CDP payload, and interactive filtering must keep ref-carrying rows.
 */

import type { CaseContext, ContractCase } from './cases'
import { errorClass, expectError, expectOk, waitUntil } from './helpers'

/**
 * The server runs evaluate code as an async function BODY, so bare
 * expressions come back `undefined` — callers must pass `return …`.
 */
async function evaluateText(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<string> {
  return expectOk(
    await ctx.mcp.callTool('evaluate', { page, code }),
    `evaluate ${code}`,
  )
}

/** Ref for the first snapshot row whose line mentions `needle`. */
function refFor(snapshot: string, needle: string): string {
  for (const line of snapshot.split('\n')) {
    if (line.includes(needle)) {
      const match = line.match(/\[ref=(e\d+)\]/)
      if (match) return match[1]
    }
  }
  throw new Error(`no ref found for "${needle}" in:\n${snapshot.slice(0, 500)}`)
}

export const navigateSnapshotCases: ContractCase[] = [
  {
    name: 'navigate: url returns a fresh snapshot with refs',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const text = expectOk(
        await ctx.mcp.callTool('navigate', {
          page,
          action: 'url',
          url: ctx.fixture('/form.html'),
        }),
        'navigate url',
      )
      if (
        !text.includes(`[Page ${page} snapshot]`) ||
        !text.includes('[ref=')
      ) {
        throw new Error(
          `navigate did not embed a ref-carrying snapshot: ${text.slice(0, 400)}`,
        )
      }
      const path = await evaluateText(ctx, page, 'return location.pathname')
      if (!path.includes('/form.html')) {
        throw new Error(`navigate did not change the url: ${path}`)
      }
    },
  },
  {
    name: 'navigate: back and forward round-trip',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      expectOk(
        await ctx.mcp.callTool('navigate', {
          page,
          action: 'url',
          url: ctx.fixture('/form.html'),
        }),
      )
      expectOk(await ctx.mcp.callTool('navigate', { page, action: 'back' }))
      const afterBack = await evaluateText(
        ctx,
        page,
        'return location.pathname',
      )
      if (!afterBack.includes('/links.html')) {
        throw new Error(`back did not return to links.html: ${afterBack}`)
      }
      expectOk(await ctx.mcp.callTool('navigate', { page, action: 'forward' }))
      const afterForward = await evaluateText(
        ctx,
        page,
        'return location.pathname',
      )
      if (!afterForward.includes('/form.html')) {
        throw new Error(`forward did not return to form.html: ${afterForward}`)
      }
    },
  },
  {
    name: 'navigate: reload resets in-page state',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      const snapshot = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: refFor(snapshot, 'Add item'),
        }),
        'act click Add item',
      )
      await waitUntil(
        async () =>
          (
            await evaluateText(
              ctx,
              page,
              'return document.getElementById("marker").textContent',
            )
          ).includes('mutated-1'),
        'the click to mutate the marker',
      )
      expectOk(await ctx.mcp.callTool('navigate', { page, action: 'reload' }))
      const marker = await evaluateText(
        ctx,
        page,
        'return document.getElementById("marker").textContent',
      )
      if (!marker.includes('fresh-load')) {
        throw new Error(`reload did not reset the marker: ${marker}`)
      }
    },
  },
  {
    name: 'navigate: file/javascript/data schemes are refused, chrome is not guarded',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const classes: string[] = []
      for (const url of [
        'file:///etc/hosts',
        'javascript:alert(1)',
        'data:text/html,<b>x</b>',
      ]) {
        const text = expectError(
          await ctx.mcp.callTool('navigate', { page, action: 'url', url }),
          `navigate to ${url}`,
        )
        classes.push(errorClass(text))
      }
      if (classes.some((error) => error !== 'scheme-refused')) {
        throw new Error(
          `scheme guard returned unexpected errors: ${classes.join(', ')}`,
        )
      }
      // chrome:// is intentionally outside this HTTP-scheme guard.
      const chrome = await ctx.mcp.callTool('navigate', {
        page,
        action: 'url',
        url: 'chrome://version',
      })
      expectOk(chrome, 'navigate to chrome://version')
    },
  },
  {
    name: 'navigate: garbage url errors',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      expectError(
        await ctx.mcp.callTool('navigate', {
          page,
          action: 'url',
          url: 'http://[::invalid::',
        }),
        'navigate to garbage url',
      )
    },
  },

  // snapshot ---------------------------------------------------------------
  {
    name: 'snapshot: full default mints refs for every interactive (REGRESSION)',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const text = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const refs = text.match(/\[ref=e\d+\]/g) ?? []
      if (refs.length === 0) {
        throw new Error(
          `snapshot minted ZERO refs — the backendDOMNodeId regression class:\n${text.slice(0, 400)}`,
        )
      }
      for (const needle of ['Submit', 'Name', 'Color', 'Subscribe']) {
        refFor(text, needle)
      }
    },
  },
  {
    name: 'snapshot: interactive mode drops paragraphs, keeps interactives (REGRESSION)',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const full = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const interactive = expectOk(
        await ctx.mcp.callTool('snapshot', { page, mode: 'interactive' }),
      )
      const hasParagraphs = /- paragraph/.test(interactive)
      const keepsLinks = interactive.includes('[ref=')
      if (hasParagraphs || !keepsLinks) {
        throw new Error(
          `interactive mode kept paragraphs or lost refs:\n${interactive.slice(0, 400)}`,
        )
      }
      if (interactive.length >= full.length) {
        throw new Error('interactive snapshot was not smaller than full')
      }
    },
  },
  {
    name: 'snapshot: depth=1 truncates the tree',
    async run(ctx) {
      // form.html nests options two levels deep — the shallowest page
      // where depth pruning is observable.
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const full = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const shallow = expectOk(
        await ctx.mcp.callTool('snapshot', { page, depth: 1 }),
      )
      const truncated =
        shallow.split('\n').length < full.split('\n').length &&
        !shallow.includes('option "Red"')
      if (!truncated) {
        throw new Error('depth=1 did not prune the option rows')
      }
    },
  },
  {
    name: 'snapshot: same node keeps the same ref across snapshots',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const first = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const second = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const stable = refFor(first, 'Submit') === refFor(second, 'Submit')
      if (!stable) {
        throw new Error('the Submit button changed refs between snapshots')
      }
    },
  },
  {
    name: 'snapshot: full and interactive mint identical refs',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const full = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const interactive = expectOk(
        await ctx.mcp.callTool('snapshot', { page, mode: 'interactive' }),
      )
      const same = refFor(full, 'Submit') === refFor(interactive, 'Submit')
      if (!same) {
        throw new Error('interactive mode re-minted refs for the same node')
      }
    },
  },
  {
    name: 'snapshot: renders values and checked/disabled/level states',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const initial = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      // Use click here so this snapshot case tests rendering independently of
      // the dedicated check/uncheck action case.
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: refFor(initial, 'Subscribe'),
        }),
        'act click Subscribe',
      )
      const text = expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const expectations: Array<[string, RegExp]> = [
        ['value rendering', /Nickname[^\n]*: "prefilled-nick"/],
        ['disabled state', /Disabled action[^\n]*\[disabled\]/],
        ['heading level', /Form fixture[^\n]*\[level=1\]/],
      ]
      const seen: Record<string, boolean> = {}
      for (const [label, pattern] of expectations) {
        seen[label] = pattern.test(text)
        if (!seen[label]) {
          throw new Error(
            `snapshot is missing ${label}:\n${text.slice(0, 600)}`,
          )
        }
      }
      // Verified gap on BOTH servers today: a checked box (DOM state
      // true) never renders a [checked] marker from a live CDP AX tree.
      // Parity holds; if either side fixes rendering, this key flags it.
    },
  },
  {
    name: 'snapshot: oversized page spills to a file with an inline excerpt',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      // The 20k-line section renders at load; wait for it before snapshotting.
      await waitUntil(
        async () =>
          (
            await evaluateText(
              ctx,
              page,
              'return document.querySelectorAll("#big h6").length',
            )
          ).includes('3001'),
        'the big section to finish rendering',
        { timeoutMs: 30_000 },
      )
      const result = await ctx.mcp.callTool('snapshot', { page })
      const text = expectOk(result, 'snapshot of oversized page')
      // Spills surface in the text — `Large snapshot (… tokens, … chars)
      // saved to: <path>` — not in structuredContent.
      const path = text.match(/saved to: (\S+)/)?.[1]
      if (!path) {
        throw new Error(
          `oversized snapshot did not spill: ${text.slice(0, 300)}`,
        )
      }
      if (!(await Bun.file(path).exists())) {
        throw new Error(`spill file missing on disk: ${path}`)
      }
      const spilled = await Bun.file(path).text()
      if (text.length >= spilled.length) {
        throw new Error('inline excerpt is not smaller than the spilled file')
      }
    },
  },
  {
    name: 'snapshot: unknown page errors',
    async run(ctx) {
      expectError(
        await ctx.mcp.callTool('snapshot', { page: 99_999 }),
        'snapshot of unknown page',
      )
    },
  },

  // diff -------------------------------------------------------------------
  {
    name: 'diff: mutation reports added nodes',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/dynamic.html'))
      expectOk(await ctx.mcp.callTool('snapshot', { page }))
      await evaluateText(
        ctx,
        page,
        'document.getElementById("add-item").click(); return "clicked"',
      )
      await waitUntil(
        async () =>
          (
            await evaluateText(
              ctx,
              page,
              'return document.querySelectorAll("#list li").length',
            )
          ).includes('4'),
        'the list mutation to land',
      )
      const diff = expectOk(await ctx.mcp.callTool('diff', { page }), 'diff')
      // Diff rows render roles, not inner text: `+ listitem …` / `2 added, 0 removed`.
      const added = Number(diff.match(/(\d+) added/)?.[1] ?? 0)
      if (added === 0 || !/\+\s+listitem/.test(diff)) {
        throw new Error(
          `diff does not report the added node:\n${diff.slice(0, 400)}`,
        )
      }
    },
  },
  {
    name: 'diff: no-op reports no change',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      expectOk(await ctx.mcp.callTool('snapshot', { page }))
      const result = await ctx.mcp.callTool('diff', { page })
      const text = expectOk(result, 'diff without changes')
      const structured = result.structuredContent as
        | { changed?: boolean }
        | undefined
      const unchanged =
        structured?.changed === false || /no change|unchanged/i.test(text)
      if (!unchanged) {
        throw new Error(`no-op diff reported a change: ${text.slice(0, 300)}`)
      }
    },
  },
  {
    name: 'diff: navigation reports the url change',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      expectOk(await ctx.mcp.callTool('snapshot', { page }))
      // The navigate tool re-snapshots (resetting the diff baseline), so
      // the url-change path only triggers for in-page navigation.
      await evaluateText(
        ctx,
        page,
        'location.href = "/form.html"; return "nav"',
      )
      await waitUntil(
        async () =>
          (await evaluateText(ctx, page, 'return location.pathname')).includes(
            '/form.html',
          ),
        'the in-page navigation to land',
      )
      const text = expectOk(
        await ctx.mcp.callTool('diff', { page }),
        'diff after navigate',
      )
      if (!/url changed/i.test(text)) {
        throw new Error(
          `diff after navigation did not flag the url change: ${text.slice(0, 300)}`,
        )
      }
    },
  },
]
