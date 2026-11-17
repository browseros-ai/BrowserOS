import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { useState } from 'react'
import type { RunDetail, RunSummary } from '../../shared/api'
import { fire, installDom, type TestDom, typeInto } from '../test-support/dom'
import { operation, record } from '../test-support/operations'
import { runDetail, runSummary } from '../test-support/runs'
import { FullOutput } from './FullOutput'
import type { OperationSelection } from './operations'

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
        operation={null}
        onOperation={() => {}}
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

describe('the full output pop-up on a call', () => {
  // One line in a loop: two whole-page snapshots, each followed by Neo's record of its text.
  const texts = [
    '- heading "First" [level=1]',
    '- heading "Second" [level=1]\n- link "More" [ref=e9]',
  ]
  const run = runSummary(8, {
    code: 'for (const i of [1, 2]) shots.push(await page.ariaSnapshot({ text: true }))',
    reply: '"done"',
  })
  const operations = [4, 6].flatMap((seq, index) => [
    operation(seq, 'page.ariaSnapshot', {
      line: 1,
      result: { document: 3, partial: false, json: [{ $more: 12 }] },
    }),
    operation(seq + 1, 'page.snapshotPending', {
      line: 1,
      args: { document: 3, text: texts[index], form: 'text', delta: false },
    }),
  ])
  const details = new Map<string, RunDetail>([
    [run.id, { ...runDetail(run, '"done"'), neo: record(operations) }],
  ])
  let dom: TestDom
  let chosen: OperationSelection[]

  function Harness() {
    const [shown, setShown] = useState<OperationSelection>({
      runId: run.id,
      operationId: 'neo-run.6',
      place: { line: 1, call: 2, calls: 2 },
    })
    return (
      <FullOutput
        run={run}
        runs={[run]}
        onRun={() => {}}
        operation={shown}
        onOperation={(next) => {
          chosen.push(next)
          setShown(next)
        }}
        onClose={() => {}}
      />
    )
  }

  beforeEach(async () => {
    dom = await installDom(details)
    chosen = []
    await dom.render(<Harness />)
  })
  afterEach(() => dom.restore())

  const text = () => dom.container.querySelector('dialog')?.textContent ?? ''
  const button = (name: string) =>
    [...dom.container.querySelectorAll('button')].find(
      (candidate) => candidate.textContent === name,
    ) as Element

  it('shows the text the script got, where the call sits, and says where it comes from', () => {
    expect(text()).toContain('page.ariaSnapshot')
    expect(text()).toContain('run 8 · line 1 · call 2 of 2')
    expect(text()).toContain('1- heading "Second" [level=1]')
    expect(text()).toContain('2- link "More" [ref=e9]')
    expect(text()).toContain('2 lines')
    expect(text()).toContain(
      "The snapshot text the script's await got, as the run rendered it",
    )
  })

  it("shows Neo's recorded tree under Raw, and steps to the previous call", async () => {
    await dom.interact(() => fire(button('Raw'), 'click'))
    expect(text()).toContain('"document": 3')
    expect(text()).toContain("Neo's record of the result, as JSON.")
    await dom.interact(() => fire(button('Previous call'), 'click'))
    expect(chosen.at(-1)?.operationId).toBe('neo-run.5')
    expect(text()).toContain('page.snapshotPending')
    expect(text()).toContain('Neo internal')
  })
})

describe('the full output pop-up on a call not read yet', () => {
  const run = runSummary(9, { reply: '"done"' })
  const details = new Map<string, RunDetail>([
    // Neo's record has no such call (or has not been read): no reply in its place.
    [run.id, { ...runDetail(run, '"done"'), neo: record([]) }],
  ])
  let dom: TestDom

  beforeEach(async () => {
    dom = await installDom(details)
    await dom.render(
      <FullOutput
        run={run}
        runs={[run, runSummary(10, { reply: '"next"' })]}
        onRun={() => {}}
        operation={{
          runId: run.id,
          operationId: 'neo-run.42',
          place: { line: 3, call: 1, calls: 1 },
        }}
        onOperation={() => {}}
        onClose={() => {}}
      />,
    )
  })
  afterEach(() => dom.restore())

  it('stays a call: says so, shows no reply, and does not step to other runs', () => {
    const text = dom.container.querySelector('dialog')?.textContent ?? ''
    expect(text).toContain('run 9 · line 3')
    expect(text).toContain("This call is not in Neo's record of the run.")
    expect(text).not.toContain('"done"')
    const buttons = [...dom.container.querySelectorAll('button')]
    const next = buttons.find((button) => button.textContent === 'Next call')
    expect(next?.hasAttribute('disabled')).toBe(true)
    expect(buttons.some((button) => button.textContent === 'Next output')).toBe(
      false,
    )
  })
})
