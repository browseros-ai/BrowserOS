/** Navigation parity: network outcomes and the browser's actual history are
 * observable contracts, including same-document entries that never reload. */
import { expect } from 'bun:test'
import type { CaseContext, ContractCase } from './cases'
import { expectOk } from './helpers'
import { errorIncludes, evalValue, runValue } from './sdk-helpers'

type Surface = 'navigate' | 'run'

async function goto(
  ctx: CaseContext,
  page: number,
  surface: Surface,
  url: string,
) {
  if (surface === 'navigate') {
    const result = await ctx.mcp.callTool('navigate', {
      page,
      action: 'url',
      url,
    })
    expectOk(result)
    return result.structuredContent
  }
  return runValue(ctx, page, `return await p.goto(${JSON.stringify(url)})`)
}

async function step(
  ctx: CaseContext,
  page: number,
  surface: Surface,
  direction: 'back' | 'forward',
) {
  if (surface === 'navigate') {
    expectOk(await ctx.mcp.callTool('navigate', { page, action: direction }))
  } else {
    expect(await runValue(ctx, page, `return await p.${direction}()`)).toEqual({
      navigated: true,
    })
  }
}

export const sdkNavigationCases: ContractCase[] = (
  ['navigate', 'run'] as const
).flatMap((surface) => [
  {
    name: `${surface}: goto names a DNS load failure`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const result =
        surface === 'navigate'
          ? await ctx.mcp.callTool('navigate', {
              page,
              action: 'url',
              url: 'https://nowhere.invalid/',
            })
          : await ctx.mcp.callTool('run', {
              code: `await browser.page(${page}).goto('https://nowhere.invalid/')`,
            })
      errorIncludes(result, 'net::ERR_NAME_NOT_RESOLVED')
    },
  },
  {
    name: `${surface}: goto reports a download and preserves the document and URL`,
    async run(ctx: CaseContext) {
      const url = ctx.fixture('/links.html')
      const page = await ctx.openPage(url)
      await evalValue(
        ctx,
        page,
        'window.navigationSentinel = "same-document"; return true',
      )
      const downloadUrl = ctx.fixture('/files/report.txt')
      if (surface === 'navigate') {
        const result = await ctx.mcp.callTool('navigate', {
          page,
          action: 'url',
          url: downloadUrl,
        })
        expect(expectOk(result)).toContain('downloaded')
      } else {
        expect(await goto(ctx, page, surface, downloadUrl)).toEqual({
          download: true,
        })
      }
      expect(
        await evalValue(
          ctx,
          page,
          'return { url: location.href, sentinel: window.navigationSentinel }',
        ),
      ).toEqual({ url, sentinel: 'same-document' })
    },
  },
  ...([200, 404] as const).map((status) => ({
    name: `${surface}: goto ${surface === 'navigate' ? 'reports' : 'returns'} HTTP ${status}`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/links.html'))
      const url = ctx.fixture(
        status === 200 ? '/form.html' : '/sdk-missing.html',
      )
      if (surface === 'navigate') {
        const result = await ctx.mcp.callTool('navigate', {
          page,
          action: 'url',
          url,
        })
        // The hosted wire drops structured content for tools without an output schema, so
        // the text is the contract: an error status is named, a 200 adds nothing.
        const text = expectOk(result)
        if (status === 404) expect(text).toContain('(HTTP 404)')
        else expect(text).not.toContain('(HTTP')
      } else {
        expect(await goto(ctx, page, surface, url)).toEqual({ status })
      }
      expect(await evalValue(ctx, page, 'return location.href')).toBe(url)
    },
  })),
  {
    name: `${surface}: back and forward with no history leave a fresh tab unchanged`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage('about:blank')
      for (const direction of ['back', 'forward'] as const) {
        if (surface === 'navigate') {
          const result = await ctx.mcp.callTool('navigate', {
            page,
            action: direction,
          })
          expect(expectOk(result)).toContain(
            `There is no ${direction} history entry`,
          )
        } else {
          expect(
            await runValue(ctx, page, `return await p.${direction}()`),
          ).toEqual({ navigated: false })
        }
        expect(await evalValue(ctx, page, 'return location.href')).toBe(
          'about:blank',
        )
      }
    },
  },
  {
    name: `${surface}: back and forward step between fixture documents`,
    async run(ctx: CaseContext) {
      const first = ctx.fixture('/links.html')
      const second = ctx.fixture('/form.html')
      const page = await ctx.openPage(first)
      await goto(ctx, page, surface, second)
      await step(ctx, page, surface, 'back')
      expect(await evalValue(ctx, page, 'return location.href')).toBe(first)
      await step(ctx, page, surface, 'forward')
      expect(await evalValue(ctx, page, 'return location.href')).toBe(second)
    },
  },
  {
    name: `${surface}: back crosses pushState without waiting for a new document`,
    async run(ctx: CaseContext) {
      const url = ctx.fixture('/links.html')
      const page = await ctx.openPage(url)
      await evalValue(
        ctx,
        page,
        'window.historySentinel = "same-document"; history.pushState({ fixture: true }, "", "?sdk-history=1"); return true',
      )
      const started = performance.now()
      await step(ctx, page, surface, 'back')
      expect(performance.now() - started).toBeLessThan(5_000)
      expect(
        await evalValue(
          ctx,
          page,
          'return { url: location.href, sentinel: window.historySentinel }',
        ),
      ).toEqual({ url, sentinel: 'same-document' })
    },
  },
  {
    name: `${surface}: goto the URL an SPA routed to returns without a new document`,
    async run(ctx: CaseContext) {
      const page = await ctx.openPage(ctx.fixture('/sdk-spa-route.html'))
      const routed = ctx.fixture('/sdk-spa-route.html#/home')
      expect(await evalValue(ctx, page, 'return location.href')).toBe(routed)
      const started = performance.now()
      const result = await goto(ctx, page, surface, routed)
      expect(performance.now() - started).toBeLessThan(5_000)
      // The status is still the document's own, read from Navigation Timing.
      if (surface === 'run') expect(result).toEqual({ status: 200 })
      expect(await evalValue(ctx, page, 'return location.href')).toBe(routed)
    },
  },
])
