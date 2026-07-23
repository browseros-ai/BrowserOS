import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { PlaybackTransport } from './PlaybackTransport'
import { usePlayback } from './use-playback'

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement

function TransportHarness() {
  const playback = usePlayback(10)
  return (
    <PlaybackTransport
      playback={playback}
      totalSeconds={10}
      frames={[]}
      onSeek={playback.seek}
    />
  )
}

beforeEach(async () => {
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const globals = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
  }
  for (const [name, value] of Object.entries(globals)) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value,
    })
  }
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })

  container = dom.document.getElementById('root') as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  root = createRoot(container)
})

afterEach(async () => {
  await act(async () => root.unmount())
  for (const [name, descriptor] of globalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, name, descriptor)
    else Reflect.deleteProperty(globalThis, name)
  }
  Reflect.deleteProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT')
})

describe('PlaybackTransport', () => {
  it('starts at 2x and keeps every speed selectable', async () => {
    await act(async () => root.render(<TransportHarness />))

    const speedButton = (label: string) =>
      [...container.querySelectorAll('button')].find(
        (button) => button.textContent === label,
      )

    expect(speedButton('1×')?.getAttribute('aria-pressed')).toBe('false')
    expect(speedButton('2×')?.getAttribute('aria-pressed')).toBe('true')
    expect(speedButton('4×')?.getAttribute('aria-pressed')).toBe('false')

    await act(async () => {
      speedButton('1×')?.dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })
    expect(speedButton('1×')?.getAttribute('aria-pressed')).toBe('true')
    expect(speedButton('2×')?.getAttribute('aria-pressed')).toBe('false')

    await act(async () => {
      speedButton('4×')?.dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })
    expect(speedButton('1×')?.getAttribute('aria-pressed')).toBe('false')
    expect(speedButton('4×')?.getAttribute('aria-pressed')).toBe('true')
  })
})
