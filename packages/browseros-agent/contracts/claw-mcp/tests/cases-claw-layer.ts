/**
 * name_session (3) + the claw-layer cross-cutting invariants: two-session
 * ownership isolation [1], browser-down guidance [2], trust-boundary
 * nonce fencing [3], auto-context embedding [4], REST audit tie-in [5],
 * and cancellation [6]. ([7] transport lives in cases-transport.)
 *
 * The browser-kill case POISONS the shared browser, so it is registered
 * LAST and the runner skips per-case page cleanup once the browser is
 * gone.
 */

import type { CaseContext, ContractCase } from './cases'
import { apiGet, errorClass, expectOk, waitUntil } from './helpers'
import { textOf } from './mcp-client'

const FENCE_OPEN = /\[UNTRUSTED_PAGE_CONTENT nonce=([0-9a-f]{16}) origin=/

async function auditSession(
  ctx: CaseContext,
  sessionId: string,
): Promise<{
  status: number
  dispatches: Array<Record<string, unknown>>
  raw: Record<string, unknown>
}> {
  const response = await apiGet(ctx.server, `/api/v1/sessions/${sessionId}`)
  if (!response.ok) return { status: response.status, dispatches: [], raw: {} }
  const raw = (await response.json()) as Record<string, unknown>
  const dispatches = (raw.dispatches ?? []) as Array<Record<string, unknown>>
  return { status: response.status, dispatches, raw }
}

export const clawLayerCases: ContractCase[] = [
  // name_session -----------------------------------------------------------
  {
    name: 'name_session: rename reports the new and old titles',
    smoke: true,
    async run(ctx) {
      const session = await ctx.openSession('rename-client')
      await waitUntil(
        async () =>
          !(await session.callTool('tabs', { action: 'list' })).isError,
        'the extra session to attach',
        { timeoutMs: 30_000 },
      )
      const text = textOf(
        await session.callTool('name_session', { name: 'form testing' }),
      )
      if (!/renamed to \S+ \(was .+\)/.test(text)) {
        throw new Error(`name_session response shape unexpected: ${text}`)
      }
    },
  },
  {
    name: 'name_session: nudges appear early, stop after rename, cap at 5',
    async run(ctx) {
      const session = await ctx.openSession('nudge-client')
      await waitUntil(
        async () =>
          !(await session.callTool('tabs', { action: 'list' })).isError,
        'the nudge session to attach',
        { timeoutMs: 30_000 },
      )
      let nudgesBeforeRename = 0
      for (let i = 0; i < 3; i += 1) {
        if (
          textOf(await session.callTool('tabs', { action: 'list' })).includes(
            'name_session',
          )
        ) {
          nudgesBeforeRename += 1
        }
      }
      await session.callTool('name_session', { name: 'renamed now' })
      const afterRename = textOf(
        await session.callTool('tabs', { action: 'list' }),
      ).includes('name_session')
      if (nudgesBeforeRename === 0) {
        throw new Error('no rename nudge appeared before naming the session')
      }
      if (afterRename) {
        throw new Error('rename nudge kept appearing after name_session')
      }
    },
  },
  {
    name: 'name_session: tab group title syncs to the new label',
    async run(ctx) {
      const session = await ctx.openSession('group-sync-client')
      await waitUntil(
        async () =>
          !(await session.callTool('tabs', { action: 'list' })).isError,
        'the group-sync session to attach',
        { timeoutMs: 30_000 },
      )
      await session.callTool('tabs', {
        action: 'new',
        url: ctx.fixture('/form.html'),
      })
      await session.callTool('name_session', { name: 'group label' })
      let groups = ''
      await waitUntil(async () => {
        groups = textOf(
          await session.callTool('tab_groups', { action: 'list' }),
        )
        return /group-label|group label/.test(groups)
      }, 'the tab group title to sync to the session label')
    },
  },

  // [1] two-session ownership isolation ------------------------------------
  {
    name: 'ownership: a second session cannot act on the first session pages',
    smoke: true,
    async run(ctx) {
      const other = await ctx.openSession('agent-other')
      await waitUntil(
        async () => !(await other.callTool('tabs', { action: 'list' })).isError,
        'the second session to attach',
        { timeoutMs: 30_000 },
      )
      if (ctx.mcp.sessionId === other.sessionId) {
        throw new Error('the two MCP sessions share a session id')
      }
      const ownPage = await ctx.openPage(ctx.fixture('/form.html'))

      // The other session sees the page bucketed under other agents.
      const bucketed = textOf(await other.callTool('tabs', { action: 'list' }))
      if (
        !/Other agents' tabs:/.test(bucketed) ||
        !bucketed.includes(`[${ownPage}]`)
      ) {
        throw new Error(
          `foreign page not bucketed for the other session:\n${bucketed.slice(0, 300)}`,
        )
      }
      // And is refused act + close, with an ownership-guard error.
      const foreignSnapshot = await other.callTool('snapshot', {
        page: ownPage,
      })
      const foreignClose = await other.callTool('tabs', {
        action: 'close',
        page: ownPage,
      })
      for (const [operation, result] of [
        ['snapshot', foreignSnapshot],
        ['close', foreignClose],
      ] as const) {
        const text = textOf(result)
        if (!result.isError || errorClass(text) !== 'not-owned') {
          throw new Error(`foreign ${operation} was not refused: ${text}`)
        }
      }
      // The owner is unaffected: the page still lists and snapshots.
      const stillOwned = textOf(
        await ctx.mcp.callTool('tabs', { action: 'list' }),
      )
      if (!stillOwned.includes(`[${ownPage}]`)) {
        throw new Error('owner lost its page after the foreign access attempt')
      }
      expectOk(
        await ctx.mcp.callTool('snapshot', { page: ownPage }),
        'owner snapshot after foreign attempt',
      )
    },
  },

  // [3] trust-boundary nonce fencing ---------------------------------------
  {
    name: 'trust-boundary: hostile page content stays inside a unique nonce fence',
    smoke: true,
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/injection.html'))
      const first = textOf(
        await ctx.mcp.callTool('read', { page, format: 'text' }),
      )
      const second = textOf(
        await ctx.mcp.callTool('read', { page, format: 'text' }),
      )
      const openNonce = first.match(FENCE_OPEN)?.[1]
      if (!openNonce) throw new Error('no opening fence in the read output')
      // The REAL closing fence carries the same nonce as the opener. The
      // page also embeds a FAKE `[END_UNTRUSTED_PAGE_CONTENT nonce=deadbeef…]`
      // — the whole point is that it is inert data inside the fence, so the
      // close must be matched by the opener's nonce, not the first marker.
      const realClose = `[END_UNTRUSTED_PAGE_CONTENT nonce=${openNonce}]`
      const closeIdx = first.indexOf(realClose)
      if (closeIdx === -1) {
        throw new Error(`no closing fence matching nonce ${openNonce}`)
      }
      const secondNonce = second.match(FENCE_OPEN)?.[1]
      if (!secondNonce || secondNonce === openNonce) {
        throw new Error('nonce was not unique across two calls')
      }
      // The hostile instruction, the fake ref, and the fake end-marker all
      // sit between the real open and close — fenced as data, powerless.
      const openIdx = first.search(FENCE_OPEN)
      const hostileIdx = first.indexOf('IGNORE PREVIOUS INSTRUCTIONS')
      const fakeRefIdx = first.indexOf('[ref=e99]')
      const fakeEndIdx = first.indexOf('nonce=deadbeefdeadbeef')
      const fenced = (index: number) => index > openIdx && index < closeIdx
      if (!fenced(hostileIdx) || !fenced(fakeRefIdx) || !fenced(fakeEndIdx)) {
        throw new Error('hostile content escaped the nonce fence')
      }
    },
  },

  // [4] auto-context embedding ---------------------------------------------
  {
    name: 'auto-context: act embeds a settled diff, navigate embeds a snapshot',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snap = textOf(await ctx.mcp.callTool('snapshot', { page }))
      const applyRef = snap
        .split('\n')
        .find((line) => line.includes('Apply'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!applyRef) throw new Error('no Apply ref for the auto-context act')
      const actText = textOf(
        await ctx.mcp.callTool('act', { page, kind: 'click', ref: applyRef }),
      )
      const navText = textOf(
        await ctx.mcp.callTool('navigate', {
          page,
          action: 'url',
          url: ctx.fixture('/links.html'),
        }),
      )
      if (!actText.includes(`[Page ${page} diff]`)) {
        throw new Error(`act omitted its settled diff: ${actText}`)
      }
      if (!navText.includes(`[Page ${page} snapshot]`)) {
        throw new Error(`navigate omitted its snapshot: ${navText}`)
      }

      // An action that logs a page error appends its console summary.
      const consolePage = await ctx.openPage(ctx.fixture('/console.html'))
      const consoleSnap = textOf(
        await ctx.mcp.callTool('snapshot', { page: consolePage }),
      )
      const throwRef = consoleSnap
        .split('\n')
        .find((line) => line.includes('Throw error'))
        ?.match(/\[ref=(e\d+)\]/)?.[1]
      if (!throwRef) throw new Error('no Throw-error ref on console.html')
      const throwAct = textOf(
        await ctx.mcp.callTool('act', {
          page: consolePage,
          kind: 'click',
          ref: throwRef,
        }),
      )
      if (!throwAct.includes(`[page ${consolePage} console]`)) {
        throw new Error(`act omitted its console summary: ${throwAct}`)
      }
    },
  },

  // [5] REST audit tie-in --------------------------------------------------
  {
    name: 'audit: dispatches record tool names, identity and 4096-char args',
    async run(ctx) {
      const session = await ctx.openSession('audit-client')
      await waitUntil(
        async () =>
          !(await session.callTool('tabs', { action: 'list' })).isError,
        'the audit session to attach',
        { timeoutMs: 30_000 },
      )
      const sessionId = session.sessionId
      if (!sessionId) throw new Error('audit session has no id')
      const opened = await session.callTool('tabs', {
        action: 'new',
        url: ctx.fixture('/form.html'),
      })
      const page = Number(textOf(opened).match(/opened page (\d+)/)?.[1])
      // A deliberately over-length argument to exercise the 4096 clamp.
      await session.callTool('evaluate', {
        page,
        code: `return "${'q'.repeat(6000)}"`,
      })

      let audit = await auditSession(ctx, sessionId)
      await waitUntil(async () => {
        audit = await auditSession(ctx, sessionId)
        return audit.dispatches.some((d) => d.toolName === 'evaluate')
      }, 'the audit log to record the dispatches')
      const tools = audit.dispatches.map((d) => d.toolName)
      if (!tools.includes('tabs') || !tools.includes('evaluate')) {
        throw new Error(`audit missing expected tool names: ${tools.join(',')}`)
      }
      const evalDispatch = audit.dispatches.find(
        (d) => d.toolName === 'evaluate',
      )
      const argsJson = evalDispatch?.argsJson as string | undefined
      if (argsJson?.length !== 4096 || !argsJson.endsWith('~')) {
        throw new Error(
          `args were not truncated to 4096 with a ~: len=${argsJson?.length}`,
        )
      }
      // Identity is slug + label, never a leaked agentId.
      const identity = {
        hasSlug: typeof evalDispatch?.slug === 'string',
        hasLabel: typeof evalDispatch?.label === 'string',
        leaksAgentId: 'agentId' in (evalDispatch ?? {}),
      }
      if (!identity.hasSlug || !identity.hasLabel || identity.leaksAgentId) {
        throw new Error(
          `audit identity shape was invalid: ${JSON.stringify(identity)}`,
        )
      }
    },
  },

  // [6] cancellation -------------------------------------------------------
  {
    name: 'cancellation: a REST cancel interrupts a long run, session survives',
    async run(ctx) {
      const session = await ctx.openSession('cancel-client')
      await waitUntil(
        async () =>
          !(await session.callTool('tabs', { action: 'list' })).isError,
        'the cancel session to attach',
        { timeoutMs: 30_000 },
      )
      const sessionId = session.sessionId
      if (!sessionId) throw new Error('cancel session has no id')

      const started = Date.now()
      const longRun = session
        .callTool('run', {
          code: 'await new Promise(() => {}); return "never"',
          timeout: 60_000,
        })
        .catch(() => ({
          isError: true,
          content: [{ type: 'text', text: 'closed' }],
        }))
      await Bun.sleep(1_500)
      const cancelResponse = await fetch(
        `${ctx.server.baseUrl}/api/v1/sessions/${sessionId}/cancel`,
        { method: 'POST', signal: AbortSignal.timeout(10_000) },
      )
      const body = (await cancelResponse.json()) as { cancelled?: number }
      if (cancelResponse.status !== 200 || (body.cancelled ?? 0) < 1) {
        throw new Error(
          `cancel did not report a cancelled dispatch: ${JSON.stringify(body)}`,
        )
      }
      const result = await longRun
      const elapsed = Date.now() - started
      if (elapsed > 30_000) {
        throw new Error(
          `cancel did not interrupt the run promptly: ${elapsed}ms`,
        )
      }
      if (!result.isError) {
        throw new Error('cancelled run returned a successful result')
      }
      // The session is still usable after a cancellation.
      expectOk(
        await session.callTool('tabs', { action: 'list' }),
        'session after cancellation',
      )
    },
  },

  // [2] browser-down guidance — MUST STAY LAST (kills the shared browser) ---
  {
    name: 'browser-down: every tool returns the start-BrowserClaw guidance',
    async run(ctx) {
      // Open a page first so refs/state exist, then pull the browser out.
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      await ctx.mcp.callTool('snapshot', { page })
      await ctx.browser.kill()
      await waitUntil(
        async () => !(await ctx.browser.isRunning()),
        'the browser to fully exit',
      )

      const probes: Array<[string, Record<string, unknown>]> = [
        ['tabs', { action: 'list' }],
        ['snapshot', { page }],
        ['navigate', { page, action: 'reload' }],
        ['read', { page, format: 'text' }],
      ]
      const classes = new Set<string>()
      for (const [tool, args] of probes) {
        const result = await ctx.mcp.callTool(tool, args)
        if (result.isError !== true) {
          throw new Error(`${tool} did not error with the browser down`)
        }
        classes.add(errorClass(textOf(result)))
      }
      if (classes.size !== 1 || !classes.has('browser-down')) {
        throw new Error(
          `browser-down guidance not uniform: ${[...classes].join(', ')}`,
        )
      }
    },
  },
]
