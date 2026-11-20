/**
 * The working overlay: while neo acts on a page, the page shows it and holds the person's input
 * until they take over. A second CDP client on the page stands in for the person: its input
 * arrives as trusted events, the same as a real mouse and keyboard.
 */

import type { CaseContext, ContractCase } from './cases'
import { expectOk, waitUntil } from './helpers'
import { textOf } from './mcp-client'

const HOST = 'browseros-agent-overlay'

interface CdpReply {
  id?: number
  result?: Record<string, unknown>
  error?: { message: string }
}

/** A CDP client attached to one page target, outside the server: the person. */
class Person {
  #ws: WebSocket
  #next = 1
  #pending = new Map<number, (reply: CdpReply) => void>()

  private constructor(ws: WebSocket) {
    this.#ws = ws
    ws.addEventListener('message', (event) => {
      const reply = JSON.parse(String(event.data)) as CdpReply
      if (reply.id === undefined) return
      this.#pending.get(reply.id)?.(reply)
      this.#pending.delete(reply.id)
    })
  }

  static async on(ctx: CaseContext, url: string): Promise<Person> {
    const targets = (await (
      await fetch(`http://127.0.0.1:${ctx.browser.cdpPort}/json/list`)
    ).json()) as Array<{
      type: string
      url: string
      webSocketDebuggerUrl: string
    }>
    const target = targets.find(
      (entry) => entry.type === 'page' && entry.url === url,
    )
    if (!target) throw new Error(`no page target for ${url}`)
    const ws = new WebSocket(target.webSocketDebuggerUrl)
    await new Promise<void>((resolve, reject) => {
      ws.addEventListener('open', () => resolve(), { once: true })
      ws.addEventListener(
        'error',
        () => reject(new Error('CDP socket failed')),
        {
          once: true,
        },
      )
    })
    return new Person(ws)
  }

  async send(
    method: string,
    params: Record<string, unknown> = {},
  ): Promise<Record<string, unknown>> {
    const id = this.#next++
    const reply = await new Promise<CdpReply>((resolve, reject) => {
      this.#pending.set(id, resolve)
      this.#ws.send(JSON.stringify({ id, method, params }))
      setTimeout(() => reject(new Error(`CDP ${method} timed out`)), 15_000)
    })
    if (reply.error) throw new Error(`${method}: ${reply.error.message}`)
    return reply.result ?? {}
  }

  /** Reads the page in its own world, which the server never touches. */
  async read<T>(expression: string): Promise<T> {
    const reply = await this.send('Runtime.evaluate', {
      expression,
      returnByValue: true,
    })
    return (reply.result as { value: T }).value
  }

  async click(x: number, y: number): Promise<void> {
    for (const type of ['mouseMoved', 'mousePressed', 'mouseReleased']) {
      await this.send('Input.dispatchMouseEvent', {
        type,
        x,
        y,
        button: 'left',
        clickCount: 1,
      })
    }
  }

  async press(key: string, code: number): Promise<void> {
    await this.send('Input.dispatchKeyEvent', {
      type: 'rawKeyDown',
      key,
      code: key,
      windowsVirtualKeyCode: code,
    })
    await this.send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key,
      code: key,
      windowsVirtualKeyCode: code,
    })
  }

  close(): void {
    this.#ws.close()
  }
}

function refFor(snapshot: string, needle: string): string {
  for (const line of snapshot.split('\n')) {
    if (line.includes(needle)) {
      const match = line.match(/\[ref=(e\d+)\]/)
      if (match) return match[1]
    }
  }
  throw new Error(`no ref for "${needle}" in:\n${snapshot.slice(0, 500)}`)
}

/** Opens the fixture on its own URL so the person can find its target, and acts once on it. */
async function heldPage(
  ctx: CaseContext,
): Promise<{ page: number; url: string; cart: string; person: Person }> {
  const url = ctx.fixture(`/overlay-hold.html?run=${crypto.randomUUID()}`)
  const page = await ctx.openPage(url)
  const snapshot = expectOk(
    await ctx.mcp.callTool('snapshot', { page }),
    'snapshot',
  )
  const cart = refFor(snapshot, 'Add to cart')
  expectOk(
    await ctx.mcp.callTool('act', { page, kind: 'click', ref: cart }),
    'neo clicks Add to cart',
  )
  const person = await Person.on(ctx, url)
  await waitUntil(
    async () =>
      await person.read<boolean>(`!!document.querySelector('${HOST}')`),
    'the overlay to be drawn',
  )
  return { page, url, cart, person }
}

export const overlayCases: ContractCase[] = [
  {
    name: 'overlay: every act kind works while the page is held',
    async run(ctx) {
      const page = await ctx.openPage(ctx.fixture('/form.html'))
      const snapshot = expectOk(
        await ctx.mcp.callTool('snapshot', { page }),
        'form snapshot',
      )
      const act = async (args: Record<string, unknown>) =>
        expectOk(
          await ctx.mcp.callTool('act', { page, diff: 'none', ...args }),
          `act ${JSON.stringify(args)}`,
        )
      await act({ kind: 'fill', ref: refFor(snapshot, 'Name'), value: 'Ada' })
      await act({ kind: 'click', ref: refFor(snapshot, 'Bio') })
      await act({ kind: 'type', text: 'Explorer' })
      await act({ kind: 'press', key: 'Tab' })
      await act({ kind: 'check', ref: refFor(snapshot, 'Subscribe') })
      await act({
        kind: 'select',
        ref: refFor(snapshot, 'Color'),
        value: 'blue',
      })
      await act({ kind: 'hover', ref: refFor(snapshot, 'Apply') })
      await act({ kind: 'scroll', direction: 'down', amount: 1 })
      const state = expectOk(
        await ctx.mcp.callTool('evaluate_in_page', {
          page,
          code: `return [document.getElementById('name').value, document.getElementById('bio').value, document.getElementById('subscribe').checked, document.getElementById('color').value, !!document.querySelector('${HOST}')].join('|')`,
        }),
        'read the form',
      )
      if (!state.includes('Ada|Explorer|true|blue|true')) {
        throw new Error(
          `the form did not take every action, or no overlay: ${state}`,
        )
      }
    },
  },
  {
    name: 'overlay: the person cannot click or scroll a page neo is working on',
    async run(ctx) {
      const { person } = await heldPage(ctx)
      try {
        const before = await person.read<number>('window.clicks')
        await person.click(140, 150)
        await person.send('Input.dispatchMouseEvent', {
          type: 'mouseWheel',
          x: 300,
          y: 300,
          deltaX: 0,
          deltaY: 600,
        })
        await Bun.sleep(300)
        const after = await person.read<[number, number, number]>(
          '[window.clicks, window.pageClicks, window.scrollY]',
        )
        if (after[0] !== before || after[1] !== before || after[2] !== 0) {
          throw new Error(`the person's input reached the page: ${after}`)
        }
      } finally {
        person.close()
      }
    },
  },
  {
    name: 'overlay: neo never reads its own overlay',
    async run(ctx) {
      const { page, person } = await heldPage(ctx)
      person.close()
      for (const [tool, args] of [
        ['snapshot', { page }],
        ['read', { page }],
        ['grep', { page, pattern: 'Take over|working on this page' }],
      ] as const) {
        const text = textOf(await ctx.mcp.callTool(tool, args))
        if (/Take over|working on this page|Let neo finish/.test(text)) {
          throw new Error(`${tool} showed the overlay: ${text.slice(0, 600)}`)
        }
      }
    },
  },
  {
    name: 'overlay: Esc takes the page over and neo checks in before acting again',
    async run(ctx) {
      const { page, cart, person } = await heldPage(ctx)
      try {
        await person.press('Escape', 27)
        await waitUntil(
          async () =>
            !(await person.read<boolean>(
              `!!document.querySelector('${HOST}')`,
            )),
          'the overlay to leave the page',
        )
        const refused = await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: cart,
        })
        if (
          !refused.isError ||
          !textOf(refused).includes('check with the user')
        ) {
          throw new Error(
            `neo was not told the person took over: ${textOf(refused)}`,
          )
        }
        expectOk(
          await ctx.mcp.callTool('snapshot', { page }),
          'reading still works',
        )
        const before = await person.read<number>('window.clicks')
        await person.click(140, 150)
        if ((await person.read<number>('window.clicks')) !== before + 1) {
          throw new Error('the person could not use the page after taking over')
        }

        const fresh = expectOk(
          await ctx.mcp.callTool('snapshot', { page }),
          'fresh snapshot',
        )
        expectOk(
          await ctx.mcp.callTool('act', {
            page,
            kind: 'click',
            ref: refFor(fresh, 'Add to cart'),
          }),
          'neo continues once it has checked in',
        )
        await waitUntil(
          async () =>
            await person.read<boolean>(`!!document.querySelector('${HOST}')`),
          'the overlay to come back',
        )
      } finally {
        person.close()
      }
    },
  },
  {
    name: 'overlay: a page that reloads itself is held again',
    async run(ctx) {
      const { person } = await heldPage(ctx)
      try {
        await person.send('Page.reload')
        await waitUntil(
          async () =>
            await person
              .read<boolean>(`!!document.querySelector('${HOST}')`)
              .catch(() => false),
          'the overlay to come back after the reload',
        )
      } finally {
        person.close()
      }
    },
  },
  {
    name: 'overlay: a page neo only reads is never held',
    async run(ctx) {
      const url = ctx.fixture(`/overlay-hold.html?read=${crypto.randomUUID()}`)
      const page = await ctx.openPage(url)
      expectOk(await ctx.mcp.callTool('snapshot', { page }), 'snapshot')
      expectOk(await ctx.mcp.callTool('read', { page }), 'read')
      const person = await Person.on(ctx, url)
      try {
        if (await person.read<boolean>(`!!document.querySelector('${HOST}')`)) {
          throw new Error('reading a page drew the overlay on it')
        }
      } finally {
        person.close()
      }
    },
  },
]
