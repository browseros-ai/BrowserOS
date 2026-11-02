/** Real pointer delivery and explicit DOM activation must remain distinct through both APIs. */
import { expect } from 'bun:test'
import type { CaseContext, ContractCase } from './cases'
import { fieldRef } from './cases-fill'
import { expectError, expectOk, waitUntil } from './helpers'
import { evalValue, type Surface } from './sdk-helpers'

function click(
  ctx: CaseContext,
  page: number,
  surface: Surface,
  ref: string,
  method?: 'dom',
) {
  return surface === 'act'
    ? ctx.mcp.callTool('act', {
        page,
        kind: 'click',
        ref,
        method,
        diff: 'none',
      })
    : ctx.mcp.callTool('run', {
        code: `await browser.page(${page}).click(${JSON.stringify(ref)}${method ? ', { method: "dom" }' : ''})`,
      })
}

export const clickRecoveryCases: ContractCase[] = (
  ['act', 'run'] as const
).flatMap((surface) => [
  ...([undefined, 'dom'] as const).flatMap((method) => [
    {
      name: `${surface}: fieldset semantics ${method ?? 'pointer'} reaches an enabled first-legend control`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(
          ctx.fixture('/click-recovery.html?mode=first-legend'),
        )
        expect(
          await evalValue(
            ctx,
            page,
            'return document.getElementById("target").matches(":disabled")',
          ),
        ).toBe(false)
        expectOk(
          await click(
            ctx,
            page,
            surface,
            await fieldRef(ctx, page, 'Recovery target'),
            method,
          ),
        )
        expect(await evalValue(ctx, page, 'return window.events')).toEqual([
          { trusted: method !== 'dom' },
        ])
      },
    },
    {
      name: `${surface}: fieldset semantics ${method ?? 'pointer'} refuses a disabled control outside the legend`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(
          ctx.fixture('/click-recovery.html?mode=fieldset-disabled'),
        )
        expect(
          await evalValue(
            ctx,
            page,
            'return document.getElementById("target").matches(":disabled")',
          ),
        ).toBe(true)
        expect(
          expectError(
            await click(
              ctx,
              page,
              surface,
              await fieldRef(ctx, page, 'Recovery target'),
              method,
            ),
          ),
        ).toContain('disabled')
        expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
      },
    },
  ]),
  {
    name: `${surface}: click recovery review regression reaches nested closed shadow roots`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=nested-closed'),
      )
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
        ),
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: true },
      ])
    },
  },
  ...(['scale(0.5)', 'rotate(65deg)'] as const).map((transform) => ({
    name: `${surface}: click recovery review regression in a ${transform} same-process frame`,
    async run(ctx: CaseContext) {
      const lab = { ...ctx, mcp: await ctx.scopedMcp() }
      const page = await lab.openPage(
        lab.fixture('/frame-lab.html?case=single-same'),
        lab.mcp,
      )
      await waitUntil(
        async () =>
          Boolean(await evalValue(lab, page, 'return window.frameLab.ready')),
        'frame lab ready',
      )
      const ref = await fieldRef(lab, page, 'Click target single-same')
      await evalValue(
        lab,
        page,
        `const frame = document.querySelector('iframe'); frame.style.transformOrigin='0 0'; frame.style.marginLeft='150px'; frame.style.transform=${JSON.stringify(transform)}; return true`,
      )
      expectOk(await click(lab, page, surface, ref))
      await waitUntil(async () => {
        const events = await evalValue<Array<{ trusted: boolean }>>(
          lab,
          page,
          'return window.frameLab.events',
        )
        return events.length === 1 && events[0].trusted
      }, 'one trusted transformed-frame click')
    },
  })),
  {
    name: `${surface}: click recovery clips an oversized target to the visible viewport`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=large'),
      )
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
        ),
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: true },
      ])
    },
  },
  {
    name: `${surface}: click recovery never rebinds a target replaced during preparation`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=replace'),
      )
      expectError(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
        ),
      )
      expect(await evalValue(ctx, page, 'return window.replaced')).toBe(true)
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
        ),
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: true },
      ])
    },
  },
  {
    name: `${surface}: click recovery finds an exposed part of a covered center`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=partial'),
      )
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
        ),
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: true },
      ])
    },
  },
  {
    name: `${surface}: click recovery waits for transient coverage before one native click`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=transient'),
      )
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
        ),
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: true },
      ])
    },
  },
  {
    name: `${surface}: click recovery returns a usable blocker ref without activating the target`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/click-recovery.html'))
      const ref = await fieldRef(ctx, page, 'Recovery target')
      const text = expectError(await click(ctx, page, surface, ref))
      // The blocker need not have an interactive AX role. Its ref must still resolve.
      const blocker = text.match(/blocker ref[=: ]+(e\d+)/i)?.[1]
      expect(blocker).toBeDefined()
      expect(blocker).not.toBe(ref)
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
      const resolved = await ctx.mcp.callTool('run', {
        code: `const node = await browser.page(${page}).resolveRef(${JSON.stringify(blocker)});
          return (await browser.cdp('DOM.describeNode', { backendNodeId: node.backendNodeId }, node.sessionId)).node.attributes`,
      })
      expectOk(resolved)
      expect(resolved.structuredContent?.value).toContain('cover')
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Dismiss obstruction'),
        ),
      )
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
        ),
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: true },
      ])
    },
  },
  {
    name: `${surface}: click recovery uses DOM activation only when explicitly requested`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/click-recovery.html'))
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
          'dom',
        ),
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: false },
      ])
      expect(
        await evalValue(ctx, page, 'return window.pageWorldClickCalls'),
      ).toBe(0)
      expect(
        await evalValue(
          ctx,
          page,
          'return Boolean(document.getElementById("cover"))',
        ),
      ).toBe(true)
    },
  },
  {
    name: `${surface}: click recovery rejects DOM activation on a disabled control`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=disabled'),
      )
      const ref = await fieldRef(ctx, page, 'Recovery target')
      expect(
        expectError(await click(ctx, page, surface, ref, 'dom')),
      ).toContain('disabled')
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
    },
  },
  {
    name: `${surface}: click recovery rechecks coverage after hover`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=hover'),
      )
      expect(
        expectError(
          await click(
            ctx,
            page,
            surface,
            await fieldRef(ctx, page, 'Recovery target'),
          ),
        ),
      ).toContain('covered by')
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
    },
  },
  {
    name: `${surface}: click recovery recenters a target hidden behind a sticky header`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=sticky'),
      )
      expectOk(
        await click(
          ctx,
          page,
          surface,
          await fieldRef(ctx, page, 'Recovery target'),
        ),
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: true },
      ])
    },
  },
  ...(['single-same', 'single-cross', 'nested-mixed'] as const).map(
    (frameCase) => ({
      name: `${surface}: click recovery identifies a parent overlay above ${frameCase}`,
      async run(ctx: CaseContext) {
        const lab = { ...ctx, mcp: await ctx.scopedMcp() }
        const page = await lab.openPage(
          lab.fixture(`/frame-lab.html?case=${frameCase}`),
          lab.mcp,
        )
        await waitUntil(
          async () =>
            Boolean(await evalValue(lab, page, 'return window.frameLab.ready')),
          'frame lab ready',
        )
        const ref = await fieldRef(lab, page, `Click target ${frameCase}`)
        await evalValue(
          lab,
          page,
          `const cover = document.createElement('div'); cover.id='parent-cover'; cover.style.cssText='position:fixed;inset:0;z-index:999;background:#888'; document.body.append(cover); return true`,
        )
        const text = expectError(await click(lab, page, surface, ref))
        expect(text).toContain('parent-cover')
        expect(text).toMatch(/blocker ref=e\d+/)
        const blocker = text.match(/blocker ref=(e\d+)/)?.[1]
        const resolved = await lab.mcp.callTool('run', {
          code: `const node = await browser.page(${page}).resolveRef(${JSON.stringify(blocker)});
          return (await browser.cdp('DOM.describeNode', { backendNodeId: node.backendNodeId }, node.sessionId)).node.attributes`,
        })
        expectOk(resolved)
        expect(resolved.structuredContent?.value).toContain('parent-cover')
        expect(
          await evalValue(lab, page, 'return window.frameLab.events'),
        ).toEqual([])
        await evalValue(
          lab,
          page,
          'document.getElementById("parent-cover").remove(); return true',
        )
        expectOk(await click(lab, page, surface, ref))
        await waitUntil(async () => {
          const events = await evalValue<Array<{ trusted: boolean }>>(
            lab,
            page,
            'return window.frameLab.events',
          )
          return events.length === 1 && events[0].trusted
        }, 'one trusted frame click')
      },
    }),
  ),
])
