import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import { PlaybackTransport } from './PlaybackTransport'
import type { ReplayAction } from './session-replay'
import { usePlayback } from './use-playback'

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement

const bookmarks: ReplayAction[] = [
  {
    frame: {
      t: 6,
      kind: 'block',
      verb: 'read',
      node: 'blocked',
      caption: 'read: blocked',
      dispatchId: 7,
    },
    startAt: 4,
    completionAt: 6,
    trackTabId: null,
    sourceIndex: 0,
  },
  {
    frame: {
      t: 8,
      kind: 'action',
      verb: 'read',
      node: 'plain',
      caption: 'read: plain',
      dispatchId: 8,
    },
    startAt: 8,
    completionAt: 8,
    trackTabId: null,
    sourceIndex: 1,
  },
]

function TransportHarness() {
  const playback = usePlayback(10)
  return (
    <>
      <PlaybackTransport
        playback={playback}
        totalSeconds={10}
        actions={bookmarks}
        onSeek={playback.seek}
      />
      <button type="button" data-command-pause onClick={playback.pause}>
        Explicit pause
      </button>
      <button type="button" data-command-play onClick={playback.play}>
        Explicit play
      </button>
      <button
        type="button"
        data-command-finish
        onClick={() => playback.seek(10)}
      >
        Finish
      </button>
    </>
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
  // linkedom ships no animation-frame API; the global clock needs one to run.
  // Never fired here — these tests only exercise the transport's controls.
  Object.assign(dom.window, {
    requestAnimationFrame: () => 1,
    cancelAnimationFrame: () => {},
  })
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
  it('starts at 4x and keeps every speed selectable', async () => {
    await act(async () => root.render(<TransportHarness />))

    const speedButton = (label: string) =>
      [...container.querySelectorAll('button')].find(
        (button) => button.textContent === label,
      )

    expect(speedButton('1×')?.getAttribute('aria-pressed')).toBe('false')
    expect(speedButton('2×')?.getAttribute('aria-pressed')).toBe('false')
    expect(speedButton('4×')?.getAttribute('aria-pressed')).toBe('true')

    await act(async () => {
      speedButton('1×')?.dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })
    expect(speedButton('1×')?.getAttribute('aria-pressed')).toBe('true')
    expect(speedButton('4×')?.getAttribute('aria-pressed')).toBe('false')

    await act(async () => {
      speedButton('2×')?.dispatchEvent(
        new window.Event('click', { bubbles: true }),
      )
    })
    expect(speedButton('1×')?.getAttribute('aria-pressed')).toBe('false')
    expect(speedButton('2×')?.getAttribute('aria-pressed')).toBe('true')
  })

  it('supports explicit pause, resume, and restart without changing speed', async () => {
    await act(async () => root.render(<TransportHarness />))

    const click = async (selector: string) => {
      await act(async () => {
        container
          .querySelector(selector)
          ?.dispatchEvent(new window.Event('click', { bubbles: true }))
      })
    }
    const toggle = () =>
      container.querySelector('button[aria-label]')?.getAttribute('aria-label')

    const oneTimes = [...container.querySelectorAll('button')].find(
      (button) => button.textContent === '1×',
    )
    // A speed change through the real picker never pauses playback.
    await act(async () => {
      oneTimes?.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(toggle()).toBe('Pause')

    await click('[data-command-pause]')
    expect(toggle()).toBe('Play')

    await click('[data-command-play]')
    expect(toggle()).toBe('Pause')

    await click('[data-command-finish]')
    expect(toggle()).toBe('Restart playback')

    await click('[data-command-play]')
    expect(toggle()).toBe('Pause')
    expect(container.textContent).toContain('0:00 / 0:10')
    expect(oneTimes?.getAttribute('aria-pressed')).toBe('true')
  })

  it('bookmarks non-action tools at their activity start', async () => {
    await act(async () => root.render(<TransportHarness />))

    const marks = [...container.querySelectorAll('button[aria-label^="Jump"]')]
    expect(marks.map((mark) => mark.getAttribute('aria-label'))).toEqual([
      'Jump to read: blocked',
    ])
    // startAt 4 of 10 seconds, not the completion at 6.
    expect(marks[0]?.getAttribute('style')?.replace(/\s/g, '')).toContain(
      'left:40%',
    )

    await act(async () => {
      marks[0]?.dispatchEvent(new window.Event('click', { bubbles: true }))
    })
    expect(container.textContent).toContain('0:04 / 0:10')
    // The jump seeks without pausing the running playback.
    expect(
      container.querySelector('button[aria-label]')?.getAttribute('aria-label'),
    ).toBe('Pause')
  })
})
