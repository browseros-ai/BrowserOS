// Neo's ref map across snapshots and re-keys. Snapshots mint refs only on a real layout, so
// this feeds the map what the engine's ai snapshot reports (ref -> element) and queries it
// through the engine it installs, as the selector evaluator would.
import { expect, it } from 'bun:test'
import { parseHTML } from 'linkedom'
import { isInsideScope } from '../../../crates/browseros-playwright/third_party/playwright/packages/injected/src/domUtils'
import { RefMap } from '../src/probe/aria-ref'
import type { QueryEngine } from '../src/probe/world'

function setup() {
  const { document } = parseHTML(
    '<html><body><ul><li id="a">A</li><li id="b">B</li></ul><p id="c">C</p></body></html>',
  )
  const element = (id: string) => {
    const found = document.getElementById(id)
    if (!found) throw new Error(`no #${id}`)
    return found
  }
  const engines = new Map<string, QueryEngine>()
  const refs = new RefMap()
  refs.install(engines, { isInsideScope })
  const engine = engines.get('aria-ref')
  if (!engine) throw new Error('the aria-ref engine was not installed')
  const query = (ref: string, root: Node = document) =>
    refs.query(() => engine.queryAll(root, ref))
  return { document, element, refs, query }
}

it('tests that refs from earlier snapshots keep naming their elements', () => {
  const { element, refs, query } = setup()
  refs.remember(new Map([['e1', { element: element('a') }]]))
  refs.remember(new Map([['e2', { element: element('b') }]]))
  expect(query('e1')).toEqual({ elements: [element('a')], staleRef: undefined })
  expect(query('e2')).toEqual({ elements: [element('b')], staleRef: undefined })
})

it('tests that a ref answers only inside the query root', () => {
  const { element, refs, query } = setup()
  refs.remember(new Map([['e1', { element: element('a') }]]))
  expect(query('e1', element('a')).elements).toEqual([element('a')])
  expect(query('e1', element('c'))).toEqual({
    elements: [],
    staleRef: undefined,
  })
})

it('tests that an unknown ref, or one whose element left the page, is reported stale', () => {
  const { element, refs, query } = setup()
  refs.remember(new Map([['e1', { element: element('a') }]]))
  expect(query('e9')).toEqual({ elements: [], staleRef: 'e9' })
  element('a').remove()
  expect(query('e1')).toEqual({ elements: [], staleRef: 'e1' })
})

it('tests that a re-key forgets every ref and the expando the engine reprints refs from', () => {
  const { element, refs, query } = setup()
  const a = element('a')
  Reflect.set(a, '_ariaRef', { role: 'listitem', name: 'A', ref: 'e1' })
  refs.remember(new Map([['e1', { element: a }]]))
  const key = refs.frameKey(element('c'))
  refs.forget()
  expect(Reflect.has(a, '_ariaRef')).toBe(false)
  expect(query('e1')).toEqual({ elements: [], staleRef: 'e1' })
  expect(query(key)).toEqual({ elements: [], staleRef: key })
  expect(refs.frameKey(element('c'))).not.toBe(key)
})
