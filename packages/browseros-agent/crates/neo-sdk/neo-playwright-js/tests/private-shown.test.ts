// The probe's `privateShown` request on the built asset (protected-fill ruling R45): whether a
// frame holds a box the mask treats as private, not empty, painted in clear. linkedom computes
// no style, so the test supplies `-webkit-text-security` as Chromium's stylesheet sets it for
// a password box.
import { afterEach, expect, it } from 'bun:test'
import { openProbePage, type ProbePage } from './support/probe-page'

let page: ProbePage | undefined
afterEach(() => page?.close())

function booted(html: string): ProbePage {
  page = openProbePage(html)
  page.boot({ frameSeq: 1, epoch: 'a' })
  const view = page.document.defaultView
  if (!view) throw new Error('the fixture has a window')
  Reflect.set(view, 'getComputedStyle', (element: Element) => ({
    getPropertyValue: (name: string) =>
      name === '-webkit-text-security' &&
      element.getAttribute('type') === 'password'
        ? 'disc'
        : '',
  }))
  return page
}

const asked = (probe: ProbePage) => probe.call({ op: 'privateShown' })

it('tests that a private box holding a value painted in clear is shown', () => {
  const probe = booted(
    '<input type="text" autocomplete="cc-number" value="4242424242424242">',
  )
  expect(asked(probe)).toMatchObject({ kind: 'value', value: { shown: true } })
})

it('tests that a password, an empty private box and an ordinary box are not', () => {
  const probe = booted(`
    <input type="password" value="hunter2">
    <input type="text" autocomplete="one-time-code" value="">
    <input type="text" name="q" value="4242424242424242">`)
  expect(asked(probe)).toMatchObject({ kind: 'value', value: { shown: false } })
})

it('tests that a private box inside an open shadow root is found', () => {
  const probe = booted('<div id="host"></div>')
  const host = probe.document.getElementById('host')
  const shadow = host?.attachShadow({ mode: 'open' })
  const field = probe.document.createElement('input')
  field.setAttribute('type', 'text')
  field.setAttribute('autocomplete', 'cc-csc')
  field.value = '737'
  shadow?.append(field)
  expect(asked(probe)).toMatchObject({ kind: 'value', value: { shown: true } })
})

it('tests that input into a private box after the first reading is counted, so a value typed and gone again between two readings is seen', () => {
  const probe = booted(`
    <input id="card" type="text" autocomplete="cc-number" value="">
    <input id="q" type="text" name="q" value="">`)
  // No host waits for the camera's walk: nothing holds the focus and nothing was typed.
  expect(asked(probe)).toEqual({
    kind: 'value',
    value: {
      shown: false,
      inputs: 0,
      boxes: true,
    },
  })
  const card = probe.document.getElementById('card') as HTMLInputElement
  const search = probe.document.getElementById('q') as HTMLInputElement
  const view = probe.document.defaultView
  if (!view) throw new Error('the fixture has a window')
  const input = () => new view.Event('input', { bubbles: true, composed: true })
  card.value = '4242'
  card.dispatchEvent(input())
  search.value = 'shoes'
  search.dispatchEvent(input())
  card.value = ''
  // Typed and gone again: shown no more, but the count moved.
  expect(asked(probe)).toEqual({
    kind: 'value',
    value: {
      shown: false,
      inputs: 1,
      boxes: true,
    },
  })
})

it('tests that boxes is true for an empty private box and for a password box, and false where only ordinary boxes are', () => {
  const boxes = (html: string) => {
    const reply = asked(booted(html))
    page?.close()
    page = undefined
    return reply
  }
  expect(
    boxes('<input type="text" autocomplete="cc-number" value="">'),
  ).toMatchObject({ value: { boxes: true, shown: false } })
  expect(boxes('<input type="password" value="">')).toMatchObject({
    value: { boxes: true, shown: false },
  })
  expect(
    boxes('<input type="text" name="q" value="shoes"><textarea></textarea>'),
  ).toMatchObject({ value: { boxes: false, shown: false } })
})
