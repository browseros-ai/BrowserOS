import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { parseHTML } from 'linkedom'

const assets = new URL(
  '../../crates/browseros-core/src/assets/',
  import.meta.url,
)
const template = readFileSync(
  new URL('agent-overlay.js', assets),
  'utf8',
).replace('__BROWSEROS_OVERLAY_MARK__', () =>
  readFileSync(new URL('neo-mark.svg', assets), 'utf8').trim(),
)
const key = Symbol.for('browseros.agentOverlay')
const show = {
  op: 'show',
  phase: 'working',
  label: 'Clicking "Add to cart"',
  rect: { x: 10, y: 20, width: 100, height: 40 },
  resume: false,
  expiryMs: 35_000,
  finishMs: 3_000,
}

interface Reply {
  status: string
  state?: string
  events: { type: string }[]
}

/** The overlay script in a page of its own, the way the isolated world runs it. */
function page(options: { frame?: boolean } = {}) {
  const { window, document } = parseHTML(
    '<!doctype html><html><body><button>Add to cart</button></body></html>',
  )
  // linkedom's window falls back to the global object, so each page gets a window of its own
  // for the script's state.
  const world: Record<string | symbol, unknown> = {
    addEventListener: window.addEventListener.bind(window),
    removeEventListener: window.removeEventListener.bind(window),
    requestAnimationFrame: (run: () => void) => setTimeout(run, 0),
  }
  world.top = options.frame ? {} : world
  const run = (command: Record<string, unknown>): Reply =>
    runInNewContext(
      template.replace('__BROWSEROS_OVERLAY_CMD__', () =>
        JSON.stringify(command),
      ),
      {
        window: world,
        document,
        globalThis: world,
        setTimeout,
        clearTimeout,
        requestAnimationFrame: world.requestAnimationFrame,
      },
    )
  const api = () =>
    world[key] as {
      yieldInput(expiryMs: number): number | string
      resumeInput(id: number | string): boolean
      hide(): boolean
      reveal(): boolean
    }
  const hosts = () => document.querySelectorAll('browseros-agent-overlay')
  /** The person's Escape: trusted, as a real keyboard's is. */
  const pressEsc = () => {
    const event = new window.Event('keydown', { bubbles: true })
    Object.defineProperty(event, 'key', { value: 'Escape' })
    Object.defineProperty(event, 'isTrusted', { value: true })
    window.dispatchEvent(event)
  }
  return { window, document, run, api, hosts, pressEsc }
}

describe('working overlay script', () => {
  test('tests that it draws once, hidden from assistive tech and with a closed shadow root', () => {
    const { run, hosts } = page()
    expect(run(show).status).toBe('shown')
    expect(run({ ...show, phase: 'between', rect: null }).state).toBe('between')
    expect(hosts().length).toBe(1)
    const host = hosts()[0]
    expect(host?.getAttribute('aria-hidden')).toBe('true')
    expect(host?.hasAttribute('inert')).toBe(false)
    expect((host as unknown as { shadowRoot: unknown }).shadowRoot).toBeNull()
  })

  test('tests that it stays out of child frames', () => {
    const { run, hosts } = page({ frame: true })
    expect(run(show).status).toBe('skipped')
    expect(hosts().length).toBe(0)
  })

  test('tests that page scripts cannot take the page over with synthetic keys', () => {
    const { window, run } = page()
    run(show)
    const escapeKey = new window.Event('keydown', { bubbles: true })
    Object.defineProperty(escapeKey, 'key', { value: 'Escape' })
    window.dispatchEvent(escapeKey)
    const beat = run({ op: 'beat', expiryMs: 35_000 })
    expect(beat.status).toBe('working')
    expect(beat.events).toEqual([])
  })

  test('tests that neo lowers the shield only while it acts, and never once the person took over', () => {
    const { run, api, pressEsc } = page()
    expect(run({ op: 'beat', expiryMs: 35_000 }).status).toBe('absent')
    run(show)
    const id = api().yieldInput(30_000)
    expect(typeof id).toBe('number')
    expect(api().resumeInput(id)).toBe(true)
    pressEsc()
    expect(api().yieldInput(30_000)).toBe('person')
  })

  test('tests that taking over removes the overlay and tells the server', () => {
    const { run, hosts, pressEsc } = page()
    run(show)
    pressEsc()
    expect(hosts().length).toBe(0)
    const beat = run({ op: 'beat', expiryMs: 35_000 })
    expect(beat.status).toBe('person')
    expect(beat.events.map((event) => event.type)).toEqual(['take-over'])
  })

  test('tests that concurrent actions each keep the shield down until the last one ends', () => {
    const { run, api } = page()
    run(show)
    const first = api().yieldInput(30_000)
    const second = api().yieldInput(30_000)
    expect(api().resumeInput(first)).toBe(false)
    expect(api().resumeInput(second)).toBe(true)
  })

  test('tests that a cancelled action stops holding the shield down while neo keeps acting', async () => {
    const { run, api } = page()
    run(show)
    // Never resumed: its action was cancelled.
    api().yieldInput(60)
    for (let action = 0; action < 4; action += 1) {
      const id = api().yieldInput(60)
      await Bun.sleep(30)
      const shieldUp = api().resumeInput(id)
      // The cancelled action lapses 60ms after it started; later actions never extend it.
      if (action === 0) expect(shieldUp).toBe(false)
      if (action >= 2) expect(shieldUp).toBe(true)
    }
  })

  test('tests that nothing is drawn after a take over until the server resumes', () => {
    const { run, hosts, pressEsc } = page()
    run(show)
    pressEsc()
    expect(run(show).status).toBe('person')
    expect(hosts().length).toBe(0)
    const resumed = run({ ...show, resume: true })
    expect(resumed.state).toBe('working')
    expect(hosts().length).toBe(1)
  })

  test('tests that a take over the server already has is not reported again', () => {
    const { run, pressEsc } = page()
    run(show)
    pressEsc()
    // Never drained: the binding call reached the server instead.
    expect(run({ ...show, resume: true }).events).toEqual([])
  })

  test('tests that hiding for a capture is counted and undone', () => {
    const { run, api } = page()
    expect(api()).toBeUndefined()
    run(show)
    expect(api().hide()).toBe(true)
    expect(api().hide()).toBe(true)
    expect(api().reveal()).toBe(true)
    expect(api().reveal()).toBe(true)
  })

  test('tests that it removes itself when the server stops beating', async () => {
    const { run, hosts } = page()
    run({ ...show, expiryMs: 30 })
    expect(hosts().length).toBe(1)
    await Bun.sleep(80)
    expect(hosts().length).toBe(0)
  })

  test('tests that "Finished" fades and removes itself', async () => {
    const { run, hosts } = page()
    run(show)
    run({ ...show, phase: 'finished', label: '', rect: null, finishMs: 20 })
    await Bun.sleep(1_600)
    expect(hosts().length).toBe(0)
  })

  test('tests that quiet removes it at once', () => {
    const { run, hosts } = page()
    run(show)
    expect(run({ ...show, phase: 'quiet' }).status).toBe('disposed')
    expect(hosts().length).toBe(0)
  })
})
