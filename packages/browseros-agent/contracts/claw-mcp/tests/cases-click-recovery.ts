/**
 * Both APIs prefer native input and share a one-shot fallback press, which
 * reaches a covered target through whatever covers it, dialogs included.
 */
import { expect } from 'bun:test'
import type { CaseContext, ContractCase } from './cases'
import { fieldRef } from './cases-fill'
import { expectError, expectOk, waitUntil } from './helpers'
import { textOf } from './mcp-client'
import { evalValue, type Surface } from './sdk-helpers'

function click(ctx: CaseContext, page: number, surface: Surface, ref: string) {
  return surface === 'act'
    ? ctx.mcp.callTool('act', {
        page,
        kind: 'click',
        ref,
        diff: 'none',
      })
    : ctx.mcp.callTool('run', {
        code: `return await browser.page(${page}).click(${JSON.stringify(ref)})`,
      })
}

/**
 * BrowserOS builds with DOM.click press natively, with trusted events; older
 * builds fall back to the page's own untrusted click(). The probe goes to the
 * page's own session, and node 0 never exists, so it presses nothing.
 */
async function hasNativePress(
  ctx: CaseContext,
  page: number,
): Promise<boolean> {
  const result = await ctx.mcp.callTool('run', {
    code: `try { return (await browser.page(${page}).cdp('DOM.click', JSON.stringify({ backendNodeId: 0 }))).refused === 'detached' }
      catch { return false }`,
  })
  expectOk(result)
  return result.structuredContent?.value === true
}

export const clickRecoveryCases: ContractCase[] = (
  ['act', 'run'] as const
).flatMap((surface) => [
  ...(['native', 'fallback'] as const).flatMap((delivery) => [
    {
      name: `${surface}: fieldset semantics ${delivery} reaches an enabled first-legend control`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(
          ctx.fixture(
            `/click-recovery.html?mode=first-legend&cover=${delivery === 'fallback'}`,
          ),
        )
        expect(
          await evalValue(
            ctx,
            page,
            'return document.getElementById("target").matches(":disabled")',
          ),
        ).toBe(false)
        const ref = await fieldRef(ctx, page, 'Recovery target')
        const native = await hasNativePress(ctx, page)
        expectOk(await click(ctx, page, surface, ref))
        expect(await evalValue(ctx, page, 'return window.events')).toEqual([
          { trusted: delivery === 'native' || native },
        ])
      },
    },
    {
      name: `${surface}: fieldset semantics ${delivery} refuses a disabled control outside the legend`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(
          ctx.fixture(
            `/click-recovery.html?mode=fieldset-disabled&cover=${delivery === 'fallback'}`,
          ),
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
    name: `${surface}: click recovery presses a covered SVG target where BrowserOS can`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=unsupported'),
      )
      const ref = await fieldRef(ctx, page, 'Recovery target')
      const native = await hasNativePress(ctx, page)
      const result = await click(ctx, page, surface, ref)
      if (native) {
        expectOk(result)
        expect(textOf(result)).toContain('DOM fallback')
        expect(await evalValue(ctx, page, 'return window.events')).toEqual([
          { trusted: true },
        ])
      } else {
        // The page's own click() exists only on HTML elements.
        expect(expectError(result)).toContain('does not support DOM activation')
        expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
      }
    },
  },
  // run's click(ref) takes no options, so only act can ask for a click the
  // fallback press cannot reproduce.
  ...(surface === 'act'
    ? [
        {
          name: `${surface}: click recovery returns a usable blocker ref when the fallback cannot press`,
          async run(ctx: CaseContext) {
            const page = await ctx.openPage(
              ctx.fixture('/click-recovery.html?mode=unsupported'),
            )
            const ref = await fieldRef(ctx, page, 'Recovery target')
            const text = expectError(
              await ctx.mcp.callTool('act', {
                page,
                kind: 'click',
                ref,
                clickCount: 2,
                diff: 'none',
              }),
            )
            // The blocker need not have an interactive AX role. Its ref must still resolve.
            const blocker = text.match(/blocker ref[=: ]+(e\d+)/i)?.[1]
            expect(blocker).toBeDefined()
            expect(blocker).not.toBe(ref)
            expect(await evalValue(ctx, page, 'return window.events')).toEqual(
              [],
            )
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
      ]
    : []),
  {
    name: `${surface}: click recovery automatically uses one isolated DOM fallback after native preparation`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/click-recovery.html'))
      const ref = await fieldRef(ctx, page, 'Recovery target')
      const native = await hasNativePress(ctx, page)
      const result = await click(ctx, page, surface, ref)
      expectOk(result)
      expect(textOf(result)).toContain('DOM fallback')
      const feedback = {
        status: 'dispatched',
        domFallback: true,
        trusted: native,
        inert: false,
      }
      expect(
        surface === 'act'
          ? result.structuredContent?.click
          : (result.structuredContent?.value as { click?: unknown })?.click,
      ).toEqual(feedback)
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: native },
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
    name: `${surface}: click recovery refuses a disabled control before fallback`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=disabled'),
      )
      const ref = await fieldRef(ctx, page, 'Recovery target')
      expect(expectError(await click(ctx, page, surface, ref))).toContain(
        'disabled',
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
    },
  },
  {
    name: `${surface}: click recovery rechecks coverage after hover`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=hover'),
      )
      const ref = await fieldRef(ctx, page, 'Recovery target')
      const native = await hasNativePress(ctx, page)
      const result = await click(ctx, page, surface, ref)
      expectOk(result)
      expect(textOf(result)).toContain('DOM fallback')
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: native },
      ])
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
  ...(['confirm', 'chooser'] as const).map((interaction) => ({
    name: `${surface}: click recovery reports fallback when a ${interaction} interrupts acknowledgement`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/click-recovery.html'))
      const ref = await fieldRef(ctx, page, 'Recovery target')
      const native = await hasNativePress(ctx, page)
      await evalValue(
        ctx,
        page,
        interaction === 'confirm'
          ? `document.getElementById('target').onclick=()=>confirm('Fallback confirmation'); return true`
          : `const file=document.body.appendChild(document.createElement('input')); file.type='file'; file.style.display='none'; document.getElementById('target').onclick=()=>file.click(); return true`,
      )
      try {
        const result = await click(ctx, page, surface, ref)
        // act intentionally marks a pending JavaScript dialog as an error;
        // delivery feedback must survive that existing response contract.
        if (surface === 'act' && interaction === 'confirm') expectError(result)
        else expectOk(result)
        expect(textOf(result)).toContain('DOM fallback')
        const value = (
          surface === 'act'
            ? result.structuredContent
            : result.structuredContent?.value
        ) as
          | {
              click?: {
                status?: string
                domFallback?: boolean
                retryable?: boolean
              }
              dialogOpen?: boolean
              fileChooserOpen?: boolean
            }
          | undefined
        expect(value?.click?.domFallback).toBe(true)
        if (interaction === 'confirm')
          expect(value?.click?.status).toBe('unknown')
        expect(['unknown', 'dispatched']).toContain(value?.click?.status)
        if (value?.click?.status === 'unknown')
          expect(value.click.retryable).toBe(false)
        if (surface === 'run')
          expect(
            interaction === 'confirm'
              ? value?.dialogOpen
              : value?.fileChooserOpen,
          ).toBe(true)
      } finally {
        expectOk(
          await ctx.mcp.callTool(
            interaction === 'confirm' ? 'act' : 'upload',
            interaction === 'confirm'
              ? { page, kind: 'dialog_dismiss' }
              : { page, files: [] },
          ),
        )
      }
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: native },
      ])
    },
  })),
  ...(surface === 'run'
    ? [
        {
          name: 'run-legacy: click recovery reports its automatic fallback',
          async run(ctx: CaseContext) {
            const page = await ctx.openPage(ctx.fixture('/click-recovery.html'))
            const ref = await fieldRef(ctx, page, 'Recovery target')
            const native = await hasNativePress(ctx, page)
            const result = await ctx.mcp.callTool('run', {
              code: `return await browser.input(${page}).click(${JSON.stringify(ref)})`,
            })
            expectOk(result)
            expect(textOf(result)).toContain('DOM fallback')
            expect(await evalValue(ctx, page, 'return window.events')).toEqual([
              { trusted: native },
            ])
          },
        },
      ]
    : []),
  ...(['inert', 'aria-hidden'] as const).map((inactive) => ({
    name: `${surface}: click recovery reaches an active dialog beside an ${inactive} older modal`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=clear'),
      )
      const ref = await fieldRef(ctx, page, 'Recovery target')
      await evalValue(
        ctx,
        page,
        `
        const lower=document.body.appendChild(document.createElement('div'));
        lower.setAttribute('role','dialog'); lower.setAttribute('aria-modal','true');
        lower.setAttribute(${JSON.stringify(inactive)}, ${JSON.stringify(inactive === 'inert' ? '' : 'true')});
        lower.textContent='Older inactive dialog';
        const active=document.body.appendChild(document.createElement('div'));
        active.setAttribute('role','dialog'); active.setAttribute('aria-modal','true');
        active.append(document.getElementById('target')); return true;
      `,
      )
      const result = await click(ctx, page, surface, ref)
      expectOk(result)
      expect(textOf(result)).not.toContain('DOM fallback')
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([
        { trusted: true },
      ])
    },
  })),
  ...(['inert', 'modal', 'closed-modal', 'aria-modal'] as const).map(
    (mode) => ({
      name: `${surface}: click recovery presses ${mode} background controls`,
      async run(ctx: CaseContext) {
        const page = await ctx.openPage(ctx.fixture('/click-recovery.html'))
        const ref = await fieldRef(ctx, page, 'Recovery target')
        const native = await hasNativePress(ctx, page)
        const setup = {
          inert: `document.getElementById('target').inert=true`,
          modal: `const dialog=document.body.appendChild(document.createElement('dialog')); dialog.innerHTML='<button>Modal action</button>'; dialog.showModal()`,
          'closed-modal': `const root=document.body.appendChild(document.createElement('section')).attachShadow({mode:'closed'}); root.innerHTML='<dialog><button>Modal action</button></dialog>'; root.querySelector('dialog').showModal()`,
          'aria-modal': `const modal=document.body.appendChild(document.createElement('div')); modal.setAttribute('role','dialog'); modal.setAttribute('aria-modal','true'); modal.innerHTML='<button>Modal action</button>'`,
        }[mode]
        await evalValue(ctx, page, `${setup}; return true`)
        const result = await click(ctx, page, surface, ref)
        expectOk(result)
        expect(textOf(result)).toContain('DOM fallback')
        // Only the native press can tell; an ARIA-only modal leaves the page interactive.
        if (surface === 'act')
          expect(
            (result.structuredContent?.click as { inert?: boolean } | undefined)
              ?.inert,
          ).toBe(native && mode !== 'aria-modal')
        expect(await evalValue(ctx, page, 'return window.events')).toEqual([
          { trusted: native },
        ])
        expect(
          await evalValue(ctx, page, 'return window.pageWorldClickCalls'),
        ).toBe(0)
      },
    }),
  ),
  // The press focuses first; a focus handler that hides the target must still
  // stop the click, on either press.
  ...(['display', 'visibility'] as const).map((property) => ({
    name: `${surface}: click recovery refuses a target its focus handler hides (${property})`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/click-recovery.html'))
      const ref = await fieldRef(ctx, page, 'Recovery target')
      await evalValue(
        ctx,
        page,
        `const target = document.getElementById('target');
        target.addEventListener('focus', () => { target.style.${property} = ${JSON.stringify(property === 'display' ? 'none' : 'hidden')} });
        return true`,
      )
      expect(expectError(await click(ctx, page, surface, ref))).toContain(
        'not visible',
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
    },
  })),
  {
    name: `${surface}: click recovery refuses a target that is no longer visible`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=clear'),
      )
      const ref = await fieldRef(ctx, page, 'Recovery target')
      await evalValue(
        ctx,
        page,
        `document.getElementById('target').style.display='none'; return true`,
      )
      expect(expectError(await click(ctx, page, surface, ref))).toContain(
        'not visible',
      )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
    },
  },
  {
    name: `${surface}: click recovery exposes no method selector`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(
        ctx.fixture('/click-recovery.html?mode=clear'),
      )
      const ref = await fieldRef(ctx, page, 'Recovery target')
      const result =
        surface === 'act'
          ? await ctx.mcp.callTool('act', {
              page,
              kind: 'click',
              ref,
              method: 'dom',
            })
          : await ctx.mcp.callTool('run', {
              code: `await browser.page(${page}).click(${JSON.stringify(ref)},{method:'dom'})`,
            })
      expectError(result)
      if (surface === 'run')
        expectError(
          await ctx.mcp.callTool('run', {
            code: `await browser.input(${page}).click(${JSON.stringify(ref)},{method:'dom'})`,
          }),
        )
      expect(await evalValue(ctx, page, 'return window.events')).toEqual([])
    },
  },
  ...(['single-same', 'single-cross', 'nested-mixed'] as const).map(
    (frameCase) => ({
      name: `${surface}: click recovery presses through a parent modal above ${frameCase}`,
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
        const native = await hasNativePress(lab, page)
        await evalValue(
          lab,
          page,
          `const dialog=document.body.appendChild(document.createElement('dialog')); dialog.innerHTML='<button>Modal action</button>'; dialog.showModal(); return true`,
        )
        const result = await click(lab, page, surface, ref)
        expectOk(result)
        expect(textOf(result)).toContain('DOM fallback')
        if (surface === 'act')
          expect(
            (result.structuredContent?.click as { inert?: boolean } | undefined)
              ?.inert,
          ).toBe(native)
        const events = await evalValue<Array<{ trusted: boolean }>>(
          lab,
          page,
          'return window.frameLab.events',
        )
        expect(events.map((event) => event.trusted)).toEqual([native])
      },
    }),
  ),
  ...(['single-same', 'single-cross', 'nested-mixed'] as const).map(
    (frameCase) => ({
      name: `${surface}: click recovery uses fallback once under a parent overlay above ${frameCase}`,
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
        const native = await hasNativePress(lab, page)
        await evalValue(
          lab,
          page,
          `const cover = document.createElement('div'); cover.id='parent-cover'; cover.style.cssText='position:fixed;inset:0;z-index:999;background:#888'; document.body.append(cover); return true`,
        )
        const result = await click(lab, page, surface, ref)
        expectOk(result)
        expect(textOf(result)).toContain('DOM fallback')
        const fallbackEvents = await evalValue<Array<{ trusted: boolean }>>(
          lab,
          page,
          'return window.frameLab.events',
        )
        expect(fallbackEvents.map((event) => event.trusted)).toEqual([native])
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
          return (
            events.length === 2 &&
            events[0].trusted === native &&
            events[1].trusted
          )
        }, 'fallback followed by one trusted frame click')
      },
    }),
  ),
])
