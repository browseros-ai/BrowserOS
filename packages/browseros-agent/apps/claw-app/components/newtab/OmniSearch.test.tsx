import { afterEach, beforeEach, describe, expect, it, mock } from 'bun:test'
import { parseHTML } from 'linkedom'
import { act } from 'react'
import type { Root } from 'react-dom/client'
import * as _helpers from './newtab-search.helpers'

const submitted: string[] = []

mock.module('./newtab-search.helpers', () => ({
  ..._helpers,
  runOmniSearch: (raw: string) => {
    submitted.push(raw)
  },
}))

const { OmniSearch } = await import('./OmniSearch')

const globalDescriptors = new Map(
  ['window', 'document', 'navigator', 'HTMLElement', 'Node', 'Event'].map(
    (name) => [name, Object.getOwnPropertyDescriptor(globalThis, name)],
  ),
)

let root: Root
let container: HTMLElement
let win: ReturnType<typeof parseHTML>['window']

beforeEach(async () => {
  submitted.length = 0
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  win = dom.window
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

async function submit(value: string) {
  await act(async () => root.render(<OmniSearch />))
  const input = container.querySelector(
    'input[name="omnisearch"]',
  ) as HTMLInputElement
  input.value = value
  const form = container.querySelector('form')
  if (!form) throw new Error('search form missing')
  await act(async () => {
    form.dispatchEvent(
      new win.Event('submit', { bubbles: true, cancelable: true }),
    )
  })
}

describe('OmniSearch', () => {
  it('sends a URL-looking entry to the omnibox runner', async () => {
    await submit('example.com')
    expect(submitted).toEqual(['example.com'])
  })

  it('sends a free-text query to the omnibox runner', async () => {
    await submit('best coffee near me')
    expect(submitted).toEqual(['best coffee near me'])
  })
})
