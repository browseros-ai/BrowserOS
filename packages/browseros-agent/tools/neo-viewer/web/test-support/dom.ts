/**
 * A DOM for the screen's component tests: linkedom, as `apps/claw-app`'s tests use it, with React
 * rendering into it under `act`. linkedom has no layout and no `<dialog>` behaviour, so the
 * pieces the components call are stubbed here: `showModal`/`close` toggle `open` and fire
 * `close`, and `scrollIntoView` does nothing.
 */

import { parseHTML } from 'linkedom'
import { act, type ReactNode } from 'react'
import type { Root } from 'react-dom/client'
import type { RunDetail } from '../../shared/api'

const GLOBALS = [
  'window',
  'document',
  'navigator',
  'HTMLElement',
  'Node',
  'Event',
  'fetch',
  'IS_REACT_ACT_ENVIRONMENT',
] as const

export interface TestDom {
  container: HTMLElement
  render(node: ReactNode): Promise<void>
  /** Lets fetches resolve and React commit what they set. */
  settle(): Promise<void>
  /** Runs a user action (a click, a key, typing) inside `act`, then settles. */
  interact(action: () => void): Promise<void>
  /** Every path fetched, in order (with its query). */
  fetched: string[]
  restore(): Promise<void>
}

/** Installs the DOM; `/api/runs/:id` answers from `details`. */
export async function installDom(
  details: Map<string, RunDetail> = new Map(),
): Promise<TestDom> {
  const saved = new Map(
    GLOBALS.map((name) => [
      name,
      Object.getOwnPropertyDescriptor(globalThis, name),
    ]),
  )
  const dom = parseHTML(
    '<!doctype html><html><body><div id="root"></div></body></html>',
  )
  const proto = dom.window.HTMLElement.prototype as unknown as Record<
    string,
    unknown
  >
  proto.showModal = function showModal(this: HTMLElement) {
    this.setAttribute('open', '')
  }
  proto.close = function close(this: HTMLElement) {
    this.removeAttribute('open')
    this.dispatchEvent(new dom.window.Event('close'))
  }
  proto.scrollIntoView = () => {}
  // React decides once, when react-dom loads, whether the DOM has `input` events by looking
  // for `oninput` on the document; linkedom has none, and React's fallback needs IE's
  // `attachEvent`. Declaring it keeps controlled inputs on the ordinary `input` path.
  Object.assign(dom.document, { oninput: null })
  const fetched: string[] = []
  const fetch = async (input: string | URL) => {
    fetched.push(String(input))
    const path = String(input).split('?')[0] ?? ''
    const id = decodeURIComponent(path.split('/').at(-1) ?? '')
    const run = details.get(id)
    return run
      ? Response.json({ run })
      : Response.json({ error: 'no such run' }, { status: 404 })
  }
  const values: Record<(typeof GLOBALS)[number], unknown> = {
    window: dom.window,
    document: dom.document,
    navigator: dom.window.navigator,
    HTMLElement: dom.window.HTMLElement,
    Node: dom.window.Node,
    Event: dom.window.Event,
    fetch,
    IS_REACT_ACT_ENVIRONMENT: true,
  }
  for (const name of GLOBALS) {
    Object.defineProperty(globalThis, name, {
      configurable: true,
      writable: true,
      value: values[name],
    })
  }
  const container = dom.document.getElementById(
    'root',
  ) as unknown as HTMLElement
  const { createRoot } = await import('react-dom/client')
  const root: Root = createRoot(container)
  const settle = async () => {
    for (let i = 0; i < 3; i++) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 0))
      })
    }
  }
  return {
    container,
    async render(node) {
      await act(async () => root.render(node))
      await settle()
    },
    settle,
    fetched,
    async interact(action) {
      await act(async () => action())
      await settle()
    },
    async restore() {
      await act(async () => root.unmount())
      for (const [name, descriptor] of saved) {
        if (descriptor) Object.defineProperty(globalThis, name, descriptor)
        else delete (globalThis as Record<string, unknown>)[name]
      }
    },
  }
}

/** A bubbling event of `type`, carrying `fields` (a key, modifiers) as React reads them. */
export function fire(
  target: Element,
  type: string,
  fields: Record<string, unknown> = {},
): void {
  const event = new window.Event(type, { bubbles: true, cancelable: true })
  Object.assign(event, fields)
  target.dispatchEvent(event)
}

/** Types into a React-controlled input: the native value setter, then `input`. */
export function typeInto(input: HTMLInputElement, text: string): void {
  const setter = Object.getOwnPropertyDescriptor(
    Object.getPrototypeOf(input),
    'value',
  )?.set
  setter?.call(input, text)
  fire(input, 'input')
}
