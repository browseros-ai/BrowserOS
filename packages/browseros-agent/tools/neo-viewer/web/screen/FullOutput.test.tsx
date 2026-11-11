import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { useState } from 'react'
import type { RunDetail, RunSummary } from '../../shared/api'
import { fire, installDom, type TestDom, typeInto } from '../test-support/dom'
import { runDetail, runSummary } from '../test-support/runs'
import { FullOutput } from './FullOutput'

// What `page.ariaSnapshot({ mode: 'ai' })` returned, as Neo writes a value: one line of JSON.
const snapshot = [
  '- generic [active] [ref=e1]:',
  '  - textbox "Full name" [ref=e5]',
  '  - combobox "Country" [ref=e7]',
].join('\n')
const replyText = JSON.stringify({ snapshot, title: 'Form' })

const runs: RunSummary[] = [
  runSummary(1, { reply: '"first"' }),
  runSummary(2, { reply: replyText }),
  runSummary(3, { roundTripMs: null }),
  runSummary(4, { reply: '"fourth"' }),
]

describe('the full output pop-up', () => {
  let dom: TestDom
  let opened: number[]
  let closed: number

  const details = new Map<string, RunDetail>(
    runs.map((run) => [
      run.id,
      runDetail(run, run.id === 'run-2' ? replyText : run.replyPreview),
    ]),
  )

  function Harness() {
    const [shown, setShown] = useState(runs[1] as RunSummary)
    return (
      <FullOutput
        run={shown}
        runs={runs}
        onRun={(run) => {
          opened.push(run.seq)
          setShown(run)
        }}
        onClose={() => {
          closed++
        }}
      />
    )
  }

  beforeEach(async () => {
    dom = await installDom(details)
    opened = []
    closed = 0
    await dom.render(<Harness />)
  })
  afterEach(() => dom.restore())

  const dialog = () => dom.container.querySelector('dialog') as Element
  const search = () =>
    dom.container.querySelector(
      'input[aria-label="Find in output"]',
    ) as HTMLInputElement
  const button = (name: string) =>
    [...dom.container.querySelectorAll('button')].find(
      (candidate) =>
        candidate.textContent === name ||
        candidate.getAttribute('aria-label') === name,
    ) as Element
  const current = () =>
    dom.container.querySelector('[data-mark="current"]')?.parentElement
      ?.textContent
  const counter = () => search().parentElement?.textContent

  it('opens as a modal with the reply numbered line by line', () => {
    expect(dialog().hasAttribute('open')).toBe(true)
    const text = dialog().textContent ?? ''
    expect(text).toContain('run 2,')
    // snapshot, its three lines, then title: five lines.
    expect(text).toContain('5 lines')
    expect(text).toContain('1snapshot')
    expect(text).toContain('3  - textbox "Full name" [ref=e5]')
    expect(text).toContain('5title    "Form"')
  })

  it('finds every match and steps through them with Enter and Shift+Enter', async () => {
    await dom.interact(() => typeInto(search(), 'REF=e'))
    expect(counter()).toBe('1/3')
    expect(dom.container.querySelectorAll('[data-mark]')).toHaveLength(3)
    expect(current()).toBe('- generic [active] [ref=e1]:')

    await dom.interact(() => fire(search(), 'keydown', { key: 'Enter' }))
    expect(counter()).toBe('2/3')
    expect(current()).toBe('  - textbox "Full name" [ref=e5]')

    await dom.interact(() =>
      fire(search(), 'keydown', { key: 'Enter', shiftKey: true }),
    )
    await dom.interact(() =>
      fire(search(), 'keydown', { key: 'Enter', shiftKey: true }),
    )
    // Back past the first match wraps to the last.
    expect(counter()).toBe('3/3')
    expect(current()).toBe('  - combobox "Country" [ref=e7]')

    await dom.interact(() => typeInto(search(), 'nothing like this'))
    expect(counter()).toBe('0')
    expect(dom.container.querySelector('[data-mark]')).toBeNull()
  })

  it('searches the raw reply when Raw is chosen', async () => {
    await dom.interact(() => fire(button('Raw'), 'click'))
    await dom.interact(() => typeInto(search(), 'ref=e'))
    // The raw text is one line of JSON: the three refs are on line 1.
    expect(counter()).toBe('1/3')
    expect(dialog().textContent).toContain('1 line')
  })

  it("moves to the previous and next finished run's output, skipping one still running", async () => {
    await dom.interact(() => fire(button('Next output'), 'click'))
    expect(opened).toEqual([4])
    expect(dialog().textContent).toContain('run 4,')
    expect(button('Next output').hasAttribute('disabled')).toBe(true)

    await dom.interact(() => fire(button('Previous output'), 'click'))
    expect(opened).toEqual([4, 2])
    await dom.interact(() => fire(button('Previous output'), 'click'))
    expect(opened).toEqual([4, 2, 1])
    expect(button('Previous output').hasAttribute('disabled')).toBe(true)
  })

  it('closes from the close button', async () => {
    await dom.interact(() => fire(button('Close'), 'click'))
    expect(dialog().hasAttribute('open')).toBe(false)
    expect(closed).toBe(1)
  })
})
