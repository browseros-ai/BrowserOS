import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { planMove } from '@browseros/agent-cursor'

const assets = new URL(
  '../../crates/browseros-core/src/assets/',
  import.meta.url,
)
const template = readFileSync(new URL('agent-pointer.js', assets), 'utf8')
const stub = readFileSync(new URL('agent-cursor-stub.js', assets), 'utf8')
const command = {
  op: 'move',
  kind: 'hover',
  x: 180,
  y: 140,
  travelMs: 400,
  timeoutMs: 800,
  idleMs: 4000,
}
const key = Symbol.for('browseros.agentPointer')
const tracedStub = `${stub}
const OriginalPlayer = BrowserOSAgentCursor.MotionPlayer;
BrowserOSAgentCursor.MotionPlayer = class extends OriginalPlayer {
  moveTo(point, options) {
    globalThis.start = {...this.pos};
    globalThis.options = options;
    return super.moveTo(point, options);
  }
};`

function harness(
  options: {
    hidden?: boolean
    frame?: boolean
    unsupported?: boolean
    reduced?: boolean
    strictCsp?: boolean
  } = {},
) {
  let now = 0
  let nextId = 1
  const timers = new Map<number, { at: number; run: () => void }>()
  const frames = new Map<number, (time: number) => void>()
  const shadows: Element[] = []
  const drawing = {
    setTransform() {},
    clearRect() {},
    beginPath() {},
    moveTo() {},
    lineTo() {},
    closePath() {},
    fill() {},
    stroke() {},
    arc() {},
    save() {},
    restore() {},
    translate() {},
  }
  class Sheet {
    cssText = ''
    replaceSync(rule: string) {
      this.cssText = rule
    }
  }
  class Element {
    parent: Element | null = null
    children: Element[] = []
    attributes = new Map<string, string>()
    styles = new Map<string, string>()
    style = {
      setProperty: (name: string, value: string) =>
        this.styles.set(name, value),
    }
    clientWidth = 800
    clientHeight = 600
    width = 0
    height = 0
    adoptedStyleSheets: Sheet[] = []
    constructor(readonly tagName: string) {}
    get isConnected(): boolean {
      return this === root || !!this.parent?.isConnected
    }
    setAttribute(name: string, value: string) {
      this.attributes.set(name, value)
    }
    appendChild(child: Element) {
      this.children.push(child)
      child.parent = this
      return child
    }
    append(...children: Element[]) {
      for (const child of children) this.appendChild(child)
    }
    attachShadow(opts: { mode: string }) {
      expect(opts.mode).toBe('closed')
      const shadow = new Element('shadow')
      shadows.push(shadow)
      return shadow
    }
    getContext() {
      return drawing
    }
    remove() {
      if (this.parent)
        this.parent.children = this.parent.children.filter(
          (child) => child !== this,
        )
      this.parent = null
    }
  }
  const root = new Element('html')
  const document = {
    visibilityState: options.hidden ? 'hidden' : 'visible',
    documentElement: options.unsupported ? null : root,
    createElement: (tag: string) => {
      if (options.strictCsp && tag === 'style')
        throw new Error('inline styles blocked')
      return new Element(tag)
    },
  }
  const world = {
    document,
    devicePixelRatio: 2,
    innerWidth: 800,
    innerHeight: 600,
    performance: { now: () => now },
    matchMedia: () => ({ matches: !!options.reduced }),
    CSSStyleSheet: options.strictCsp ? Sheet : undefined,
    setTimeout: (run: () => void, ms: number) => {
      const id = nextId++
      timers.set(id, { at: now + ms, run })
      return id
    },
    clearTimeout: (id: number) => timers.delete(id),
    requestAnimationFrame: (run: (time: number) => void) => {
      const id = nextId++
      frames.set(id, run)
      return id
    },
    cancelAnimationFrame: (id: number) => frames.delete(id),
  }
  const context = { ...world, window: world, globalThis: world }
  Object.assign(world, { top: options.frame ? {} : world })
  const execute = (cmd = command, bundle = stub) =>
    runInNewContext(
      template
        .replace('__BROWSEROS_POINTER_CURSOR__', () => bundle)
        .replace('__BROWSEROS_POINTER_CMD__', () => JSON.stringify(cmd)),
      context,
    )
  const api = () => Reflect.get(world, key)
  const advance = async (ms: number) => {
    for (let elapsed = 0; elapsed < ms; elapsed += 10) {
      now += Math.min(10, ms - elapsed)
      const pending = [...frames.values()]
      frames.clear()
      for (const run of pending) run(now)
      for (const [id, timer] of [...timers])
        if (timer.at <= now && timers.delete(id)) timer.run()
      await Promise.resolve()
    }
  }
  return {
    execute,
    api,
    advance,
    root,
    shadows,
    frames,
    document,
    context,
    world,
  }
}

describe('agent pointer page expression', () => {
  test('tests that hidden and frame contexts skip before DOM work or bundle execution', async () => {
    for (const [options, reason] of [
      [{ hidden: true }, 'hidden'],
      [{ frame: true }, 'not-top'],
      [{ unsupported: true }, 'unsupported'],
    ] as const) {
      const h = harness(options)
      expect(
        await h.execute(command, 'throw new Error("bundle must not run")'),
      ).toEqual({ status: 'skipped', reason })
      expect(h.root.children).toHaveLength(0)
      expect(h.api()).toBeUndefined()
    }
  })

  test('tests that an isolated overlay arrives, hides, and tears down without a live frame loop', async () => {
    const h = harness()
    const moving = h.execute()
    await h.advance(450)
    expect(await moving).toEqual({ status: 'arrived' })
    const host = h.root.children[0]
    expect(host?.tagName).toBe('browseros-agent-pointer')
    expect(host?.attributes.get('aria-hidden')).toBe('true')
    expect(host?.attributes.has('inert')).toBe(true)
    expect(host?.styles.get('pointer-events')).toBe('none')
    expect(Object.getOwnPropertyDescriptor(h.world, key)?.enumerable).toBe(
      false,
    )
    expect(h.api().hide()).toBe(true)
    expect(h.api().show()).toBe(true)
    expect(h.api().click(220, 170)).toBe(true)
    await h.advance(4100)
    expect(h.root.children).toHaveLength(0)
    expect(h.frames.size).toBe(0)
    expect(h.api().click(0, 0)).toBe(false)
    expect(h.api().hide()).toBe(false)
    expect(h.api().show()).toBe(false)
  })

  test('tests that a stalled player times out and a failing player fails open', async () => {
    const bundle =
      'var BrowserOSAgentCursor = { MotionPlayer: class { pos = {x:0,y:0}; place(p) {this.pos=p} moveTo() {return new Promise(()=>{})} destroy() {} } };'
    const h = harness()
    const moving = h.execute({ ...command, timeoutMs: 50 }, bundle)
    await h.advance(60)
    expect(await moving).toEqual({ status: 'timeout' })
    const bad = harness()
    expect(
      await bad.execute(command, 'throw new Error("unavailable")'),
    ).toEqual({ status: 'skipped', reason: 'error' })
    expect(bad.root.children).toHaveLength(0)
  })

  test('tests that overlapping captures stay hidden until both are restored', async () => {
    const h = harness()
    const moving = h.execute()
    await h.advance(450)
    await moving
    const canvas = h.shadows[0]?.children.find(
      (child) => child.tagName === 'canvas',
    )
    expect(h.api().hide()).toBe(true)
    expect(h.api().hide()).toBe(true)
    expect(h.api().show()).toBe(true)
    expect(canvas?.styles.get('visibility')).toBe('hidden')
    expect(h.api().show()).toBe(true)
    expect(canvas?.styles.get('visibility')).toBe('visible')
    h.api().hide()
    await h.advance(4100)
    const restarted = h.execute()
    await h.advance(450)
    await restarted
    const nextCanvas = h.shadows[1]?.children.find(
      (child) => child.tagName === 'canvas',
    )
    expect(h.api().show()).toBe(true)
    expect(nextCanvas?.styles.get('visibility')).toBe('visible')
  })

  test('tests that print hiding uses CSSOM when inline styles are blocked', async () => {
    const h = harness({ strictCsp: true })
    const moving = h.execute()
    await h.advance(450)
    expect(await moving).toEqual({ status: 'arrived' })
    expect(h.shadows[0]?.adoptedStyleSheets[0]?.cssText).toContain(
      '@media print',
    )
    expect(
      h.shadows[0]?.children.some((child) => child.tagName === 'style'),
    ).toBe(false)
  })

  test('tests that the bundle executes once and a later move continues from the last tip', async () => {
    const h = harness()
    const bundle = `${stub}\nglobalThis.loads = (globalThis.loads || 0) + 1; const OriginalPlayer = BrowserOSAgentCursor.MotionPlayer; BrowserOSAgentCursor.MotionPlayer = class extends OriginalPlayer { place(p) { globalThis.placed = {...p}; super.place(p) } moveTo(p,o) { globalThis.start = {...this.pos}; globalThis.options = o; return super.moveTo(p,o) } };`
    const first = h.execute(command, bundle)
    await h.advance(450)
    await first
    const next = h.execute({ ...command, x: 380, y: 240 }, bundle)
    expect(Reflect.get(h.world, 'start')).toEqual({ x: 180, y: 140 })
    expect(Reflect.get(h.world, 'options').click).not.toBe(true)
    await h.advance(450)
    await next
    await h.advance(4100)
    const restarted = h.execute({ ...command, x: 420, y: 300 }, bundle)
    expect(Reflect.get(h.world, 'start')).toEqual({ x: 380, y: 240 })
    expect(Reflect.get(h.world, 'loads')).toBe(1)
    await h.advance(450)
    await restarted
  })

  test('tests that fixed signature arc timing stays within the travel budget', () => {
    for (const distance of [60, 500, 1500]) {
      const trajectory = planMove(
        {
          style: 'signature_arc',
          timing: 'fixed',
          glideDurationMs: command.travelMs,
        },
        { from: { x: 0, y: 0 }, to: { x: distance, y: 0 } },
      )
      expect(trajectory.arrivalT * 1000).toBeLessThanOrEqual(
        command.travelMs + 50,
      )
    }
  })

  test('tests that the first move starts 160 pixels up-left within viewport margins', async () => {
    for (const { target, expected, viewport } of [
      {
        target: { x: 400, y: 300 },
        expected: { x: 272, y: 204 },
        viewport: { width: 800, height: 600 },
      },
      {
        target: { x: 20, y: 25 },
        expected: { x: 16, y: 16 },
        viewport: { width: 800, height: 600 },
      },
      {
        target: { x: 799, y: 599 },
        expected: { x: 671, y: 503 },
        viewport: { width: 800, height: 600 },
      },
      {
        target: { x: 1200, y: 1000 },
        expected: { x: 784, y: 584 },
        viewport: { width: 800, height: 600 },
      },
      {
        target: { x: 4, y: 10 },
        expected: { x: 4, y: 10 },
        viewport: { width: 8, height: 20 },
      },
    ]) {
      const h = harness()
      h.world.innerWidth = viewport.width
      h.world.innerHeight = viewport.height
      const moving = h.execute({ ...command, ...target }, tracedStub)
      const start = Reflect.get(h.world, 'start')
      expect(start).toEqual(expected)
      if (target.x === 400)
        expect(Math.hypot(target.x - start.x, target.y - start.y)).toBe(160)
      await h.advance(450)
      expect(await moving).toEqual({ status: 'arrived' })
    }
  })

  test('tests that signature arc meets the arrival upper bound across 40–1600 pixels in both directions', async () => {
    const h = harness()
    const moving = h.execute(command, tracedStub)
    const params = Reflect.get(h.world, 'options').params
    expect(params).toEqual({
      style: 'signature_arc',
      timing: 'fixed',
      glideDurationMs: command.travelMs,
    })
    for (let distance = 40; distance <= 1600; distance++) {
      for (const direction of [1, -1]) {
        const request = {
          from: { x: 800, y: 400 },
          to: { x: 800 + direction * distance, y: 400 },
        }
        const arrivalMs = planMove(params, request).arrivalT * 1000
        expect(arrivalMs).toBeLessThanOrEqual(command.travelMs + 50)
      }
    }
    await h.advance(450)
    expect(await moving).toEqual({ status: 'arrived' })
  })

  test('tests that the real first-move path stays on screen for header targets', async () => {
    for (const y of [120, 180]) {
      const h = harness()
      h.world.innerWidth = 1280
      h.world.innerHeight = 800
      const target = { x: 640, y }
      const moving = h.execute({ ...command, ...target }, tracedStub)
      const from = Reflect.get(h.world, 'start')
      const params = Reflect.get(h.world, 'options').params
      const trajectory = planMove(params, { from, to: target })
      const samples = trajectory.samples.filter(
        (sample) => sample.t <= trajectory.arrivalT,
      )
      expect(
        samples.every(
          (point) =>
            point.x >= 0 && point.x <= 1280 && point.y >= 0 && point.y <= 800,
        ),
      ).toBe(true)
      await h.advance(450)
      expect(await moving).toEqual({ status: 'arrived' })
    }
  })
})

test('tests that the stub sizes for DPR and resolves superseded moves and placement', async () => {
  const h = harness()
  const result = runInNewContext(`${stub}; BrowserOSAgentCursor`, h.context)
  const canvas = h.document.createElement('canvas')
  const player = new result.MotionPlayer(canvas, { float: false })
  const first = player.moveTo(
    { x: 100, y: 100 },
    { params: { glideDurationMs: 400 } },
  )
  const second = player.moveTo(
    { x: 200, y: 100 },
    { params: { glideDurationMs: 400 } },
  )
  await first
  player.place({ x: 20, y: 30 })
  await second
  expect(player.pos).toEqual({ x: 20, y: 30 })
  await h.advance(20)
  expect([canvas.width, canvas.height]).toEqual([1600, 1200])
  player.destroy()
  expect(h.frames.size).toBe(0)
})
