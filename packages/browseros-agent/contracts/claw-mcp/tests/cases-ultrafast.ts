/**
 * Ultrafast skills: saving a session compiles it into a verified flow, run_skill runs it or
 * hands back with an owned page and the write guard, and a one-step patch repairs a site
 * change. Every case drives the test shop, switches ultrafast on through the settings API, and
 * switches it off again so later cases see the default tool list.
 */

import {
  startUltrafastShop,
  type UltrafastShop,
} from '../fixtures/ultrafast-shop'
import type { CaseContext, ContractCase } from './cases'
import { expectOk } from './helpers'
import { type McpSession, textOf } from './mcp-client'
import {
  SHOP_EXAMPLES,
  SHOP_PROSE,
  SHOP_TYPES,
  shopSteps,
} from './ultrafast-flows'

type Structured = Record<string, unknown> & {
  status?: string
  ultrafast?: string
  cause?: string
  reason?: string
  failed?: { step?: number }
  effects?: { committed?: unknown }
  page?: { owned?: boolean }
  brief?: { runs?: Array<{ tag?: string }>; hints?: string[] }
}

async function api(
  ctx: CaseContext,
  method: string,
  path: string,
): Promise<{ status: number; body: Record<string, unknown> }> {
  const response = await fetch(`${ctx.server.baseUrl}${path}`, { method })
  const text = await response.text()
  return { status: response.status, body: text ? JSON.parse(text) : {} }
}

async function structured(
  session: McpSession,
  tool: string,
  args: Record<string, unknown>,
): Promise<Structured> {
  const result = await session.callTool(tool, args, 150_000)
  expectOk(result, tool)
  return (result.structuredContent ?? {}) as Structured
}

async function closeMyPages(session: McpSession): Promise<void> {
  await session
    .callTool('run', {
      code: "for (const p of (await browser.pages.list()).filter((p) => p.ownership === 'mine')) await browser.page(p.pageId).close(); return true",
    })
    .catch(() => {})
}

/** Saves the shop add-to-cart skill as an ultrafast skill and approves its writes. */
async function savedShopSkill(
  ctx: CaseContext,
  session: McpSession,
  shop: UltrafastShop,
  name: string,
): Promise<void> {
  await api(ctx, 'DELETE', `/api/v1/skills/neo-${name}`)
  await structured(session, 'save_skill', {
    name,
    description: 'Add a product to the test shop cart',
    steps: SHOP_PROSE,
  })
  const verified = await structured(session, 'save_skill', {
    name,
    flow: shopSteps(shop.origin),
    effects: 'write',
    examples: SHOP_EXAMPLES,
    ...SHOP_TYPES,
  })
  if (verified.ultrafast !== 'verified') {
    throw new Error(`expected a verified flow, got ${JSON.stringify(verified)}`)
  }
  const approved = await api(
    ctx,
    'POST',
    `/api/v1/skills/neo-${name}/ultrafast/approve-write`,
  )
  if (approved.status !== 200) {
    throw new Error(`approve-write failed: ${approved.status}`)
  }
}

async function withShop(
  ctx: CaseContext,
  body: (shop: UltrafastShop, session: McpSession) => Promise<void>,
): Promise<void> {
  const shop = await startUltrafastShop()
  const session = await ctx.openSession('ultrafast-contract')
  try {
    await body(shop, session)
  } finally {
    await closeMyPages(session)
    await shop.stop()
  }
}

export const ultrafastCases: ContractCase[] = [
  {
    name: 'ultrafast: on, run_skill is listed and save_skill takes a flow',
    async run(ctx) {
      const session = await ctx.openSession('ultrafast-on')
      const tools = await session.listTools()
      if (!tools.some((tool) => tool.name === 'run_skill')) {
        throw new Error('run_skill is not listed')
      }
      const save = tools.find((tool) => tool.name === 'save_skill')
      const schema = save?.inputSchema as {
        properties?: Record<string, unknown>
      }
      if (!schema?.properties?.flow) {
        throw new Error('save_skill does not offer flow fields')
      }
    },
  },
  {
    name: 'ultrafast: saving a session compiles a verified flow without writing',
    async run(ctx) {
      await withShop(ctx, async (shop, session) => {
        await session.callTool('run', {
          code: `const p = await browser.open('${shop.url('/s?k=sensodyne')}'); await p.waitForSelector('[data-result]'); return (await p.snapshot()).text`,
        })
        const first = await structured(session, 'save_skill', {
          name: 'contract-shop-add',
          description: 'Add a product to the test shop cart',
          steps: SHOP_PROSE,
        })
        if (first.ultrafast !== 'pending' || !first.brief?.runs?.length) {
          throw new Error(
            `call 1 should return a brief: ${JSON.stringify(first)}`,
          )
        }
        const before = shop.state.cart.length
        const second = await structured(session, 'save_skill', {
          name: 'contract-shop-add',
          flow: shopSteps(shop.origin),
          effects: 'write',
          examples: SHOP_EXAMPLES,
          ...SHOP_TYPES,
        })
        if (second.ultrafast !== 'verified') {
          throw new Error(`call 2 should verify: ${JSON.stringify(second)}`)
        }
        if (shop.state.cart.length !== before) {
          throw new Error('verification wrote to the cart')
        }
        const detail = await api(
          ctx,
          'GET',
          '/api/v1/skills/neo-contract-shop-add',
        )
        const body = String(detail.body.body ?? '')
        if (!body.includes('## Run it fast')) {
          throw new Error(`SKILL.md should say Run it fast: ${body}`)
        }
      })
    },
  },
  {
    name: 'ultrafast: run_skill finishes, or hands back with an owned page and the write guard',
    async run(ctx) {
      await withShop(ctx, async (shop, session) => {
        await savedShopSkill(ctx, session, shop, 'contract-shop-run')
        shop.reset()
        const done = await structured(session, 'run_skill', {
          name: 'contract-shop-run',
          inputs: { query: 'colgate', match: 'colgate' },
        })
        if (done.status !== 'done' || shop.state.cart.length !== 1) {
          throw new Error(
            `expected done with one item: ${JSON.stringify(done)}`,
          )
        }
        shop.reset()
        shop.state.layout = 'v2'
        const before = await structured(session, 'run_skill', {
          name: 'contract-shop-run',
          inputs: { query: 'colgate', match: 'colgate' },
        })
        if (
          before.status !== 'needs_agent' ||
          before.effects?.committed !== false ||
          before.cause !== 'site_changed' ||
          before.page?.owned !== true ||
          shop.state.cart.length !== 0
        ) {
          throw new Error(
            `v2 should hand back before writing: ${JSON.stringify(before)}`,
          )
        }
        shop.reset()
        shop.state.layout = 'v3'
        const after = await structured(session, 'run_skill', {
          name: 'contract-shop-run',
          inputs: { query: 'dove', match: 'dove' },
        })
        if (
          after.status !== 'needs_agent' ||
          after.effects?.committed !== true ||
          after.failed?.step !== 4 ||
          shop.state.cart.length !== 1
        ) {
          throw new Error(
            `v3 should hand back after one write: ${JSON.stringify(after)}`,
          )
        }
      })
    },
  },
  {
    name: 'ultrafast: a one-step patch repairs a site change',
    async run(ctx) {
      await withShop(ctx, async (shop, session) => {
        await savedShopSkill(ctx, session, shop, 'contract-shop-patch')
        shop.reset()
        shop.state.layout = 'v2'
        const handed = await structured(session, 'run_skill', {
          name: 'contract-shop-patch',
          inputs: { query: 'colgate', match: 'colgate' },
        })
        if (handed.cause !== 'site_changed') {
          throw new Error(`expected a site change: ${JSON.stringify(handed)}`)
        }
        const fixed = shopSteps(shop.origin, 'Add to (?:cart|basket)')[1]
        const patched = await structured(session, 'save_skill', {
          name: 'contract-shop-patch',
          patch: { step: 2, fn: fixed?.fn },
        })
        if (patched.ultrafast !== 'verified' || patched.repaired !== true) {
          throw new Error(`the patch should repair: ${JSON.stringify(patched)}`)
        }
        shop.reset()
        shop.state.layout = 'v2'
        const done = await structured(session, 'run_skill', {
          name: 'contract-shop-patch',
          inputs: { query: 'colgate', match: 'colgate' },
        })
        if (done.status !== 'done' || shop.state.cart.length !== 1) {
          throw new Error(
            `the repaired flow should run: ${JSON.stringify(done)}`,
          )
        }
        const text = textOf(
          await session.callTool('mark_skill_run', {
            name: 'contract-shop-patch',
          }),
        )
        if (text.includes('no working ultrafast flow')) {
          throw new Error('a working flow should not be nudged')
        }
      })
    },
  },
]
