import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import type { TakeoverContext } from './takeover.types'
import {
  createTakeoverBar,
  TAKEOVER_BAR_CSS,
  type TakeoverBar,
} from './takeover-bar'

let window: ReturnType<typeof parseHTML>['window']
let root: HTMLElement
let bar: TakeoverBar | null = null

function context(over: Partial<TakeoverContext> = {}): TakeoverContext {
  return {
    sessionId: 'session-1',
    agentLabel: 'CLAUDE-CODE',
    reason: 'Enter the login so the run can continue',
    resumeHint: 'play Castlevania',
    site: 'netflix.com',
    requestedAt: Date.now() - 134_000,
    ...over,
  }
}

function click(selector: string) {
  const node = root.querySelector(selector)
  if (!node) throw new Error(`missing ${selector}`)
  node.dispatchEvent(new window.Event('click', { bubbles: true }))
}

beforeEach(() => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  window = dom.window
  root = dom.document.getElementById('root') as unknown as HTMLElement
  bar = null
})

afterEach(() => {
  bar?.destroy()
})

describe('takeover bar', () => {
  it('stays hidden until a context arrives', () => {
    bar = createTakeoverBar(root, { onHandBack: () => {} })
    expect((root.querySelector('.nt-root') as HTMLElement).hidden).toBe(true)
  })

  it('leads with BrowserOS neo branding and shows the request', () => {
    bar = createTakeoverBar(root, { onHandBack: () => {} })
    bar.update(context())

    expect((root.querySelector('.nt-root') as HTMLElement).hidden).toBe(false)
    expect(root.querySelector('.nt-brand')?.textContent).toBe('BrowserOS neo')
    expect(root.querySelector('.nt-dog svg')).not.toBeNull()
    expect(root.querySelector('.nt-agent')?.textContent).toBe('CLAUDE-CODE')
    expect(root.querySelector('.nt-site')?.textContent).toContain('netflix.com')
    expect(root.querySelector('.nt-reason')?.textContent).toContain(
      'Enter the login',
    )
    expect(root.querySelector('.nt-resume')?.textContent).toContain(
      'play Castlevania',
    )
    expect(root.querySelector('.nt-timer')?.textContent).toBe('2:14')
  })

  it('hands back with the typed note', () => {
    let handed: string | undefined
    bar = createTakeoverBar(root, {
      onHandBack: (note) => {
        handed = note
      },
    })
    bar.update(context())
    const note = root.querySelector('.nt-note') as HTMLInputElement
    note.value = '  signed in  '
    click('.nt-handback')
    expect(handed).toBe('signed in')
  })

  it('minimizes to the pill and expands again', () => {
    bar = createTakeoverBar(root, { onHandBack: () => {} })
    bar.update(context())

    click('.nt-min')
    expect((root.querySelector('.nt-bar') as HTMLElement).hidden).toBe(true)
    expect((root.querySelector('.nt-pill-btn') as HTMLElement).hidden).toBe(
      false,
    )

    click('.nt-pill-btn')
    expect((root.querySelector('.nt-bar') as HTMLElement).hidden).toBe(false)
    expect((root.querySelector('.nt-pill-btn') as HTMLElement).hidden).toBe(
      true,
    )
  })

  it('forces [hidden] so the reset display rules cannot show both states', () => {
    // all:initial plus explicit display rules outrank the UA [hidden] default,
    // so the shadow CSS must force it or minimize leaves both states visible.
    expect(TAKEOVER_BAR_CSS).toContain('[hidden] { display: none !important; }')
  })

  it('hides again when the request clears', () => {
    bar = createTakeoverBar(root, { onHandBack: () => {} })
    bar.update(context())
    bar.update(null)
    expect((root.querySelector('.nt-root') as HTMLElement).hidden).toBe(true)
  })
})
