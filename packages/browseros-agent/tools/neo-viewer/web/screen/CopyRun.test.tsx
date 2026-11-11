import { afterEach, describe, expect, it } from 'bun:test'
import {
  detailWith,
  twoLineRecord,
} from '../../server/test-support/neo-records'
import type { RunDetail } from '../../shared/api'
import { runMarkdown } from '../../shared/run-markdown'
import { fire, installDom, type TestDom } from '../test-support/dom'
import { runTotals } from './measures'
import { RunTable } from './RunTable'
import { DEFAULT_SORT } from './sort'

const REPLY = `{"title":"Example","rows":"${'r'.repeat(5_000)}"}`
const first = detailWith(1, { text: REPLY, neo: twoLineRecord() })
const second = detailWith(2, { text: '2', neo: null })

/** A clipboard that records what it was handed, as the browser's would hold it. */
function fakeClipboard() {
  const written: Array<Promise<string>> = []
  const clipboard = {
    async write(items: Array<{ items: Record<string, Promise<Blob>> }>) {
      const blob = items[0]?.items['text/plain']
      written.push(Promise.resolve(blob).then((b) => (b as Blob).text()))
    },
    async writeText(text: string) {
      written.push(Promise.resolve(text))
    },
  }
  return { clipboard, written }
}

describe('Copy on a run row', () => {
  let dom: TestDom
  const toggles: number[] = []
  const savedItem = Object.getOwnPropertyDescriptor(globalThis, 'ClipboardItem')

  let details: Map<string, RunDetail>

  function Table({ openIds }: { openIds: ReadonlySet<string> }) {
    return (
      <RunTable
        runs={[first, second]}
        totals={runTotals([first, second])}
        toolsList={null}
        sort={DEFAULT_SORT}
        onSort={() => {}}
        openIds={openIds}
        selectedId={null}
        onRow={(run) => toggles.push(run.seq)}
        onOpenFull={() => {}}
        nowMs={Date.now()}
      />
    )
  }

  async function table(openIds: ReadonlySet<string> = new Set()) {
    details = new Map<string, RunDetail>([
      [first.id, first],
      [second.id, second],
    ])
    dom = await installDom(details)
    await dom.render(<Table openIds={openIds} />)
  }

  afterEach(async () => {
    toggles.length = 0
    await dom.restore()
    if (savedItem) Object.defineProperty(globalThis, 'ClipboardItem', savedItem)
    else delete (globalThis as Record<string, unknown>).ClipboardItem
  })

  const copyButton = (seq: number) =>
    dom.container.querySelector(`[data-copy-run="${seq}"]`) as Element
  const row = (seq: number) =>
    dom.container.querySelector(`[data-run="${seq}"]`) as Element

  it('copies a closed row: it fetches the run, hands the clipboard the shared Markdown, and leaves the row closed', async () => {
    await table()
    const { clipboard, written } = fakeClipboard()
    Object.assign(navigator, { clipboard })
    // The browser's ClipboardItem takes a promise of the blob, so the write keeps the click.
    Object.defineProperty(globalThis, 'ClipboardItem', {
      configurable: true,
      value: class {
        constructor(readonly items: Record<string, Promise<Blob>>) {}
      },
    })
    // Copy is beside the row's button, never inside it.
    expect(row(1).querySelector('[data-copy-run]')).toBeNull()
    await dom.interact(() => fire(copyButton(1), 'click'))
    expect(written).toHaveLength(1)
    const text = await written[0]
    expect(text).toBe(runMarkdown(first, { replyChars: null }))
    expect(text.startsWith('# Run 001 · ')).toBe(true)
    // All of what the model read, not the export's first 2,000 characters.
    expect(text).toContain(REPLY)
    expect(text).toContain('- L1 page.goto: +10 ms, 412 ms, ok')
    expect(text).not.toContain('## Operations')
    // The click did not reach the row: nothing opened or closed.
    expect(toggles).toEqual([])
    expect(row(1).getAttribute('aria-expanded')).toBe('false')
    // A tick and "Copied", for a moment.
    expect(dom.container.querySelector('[role="status"]')?.textContent).toBe(
      'Copied',
    )
  })

  it('writes plain text where the browser has no ClipboardItem', async () => {
    await table()
    const { clipboard, written } = fakeClipboard()
    Object.assign(navigator, { clipboard })
    delete (globalThis as Record<string, unknown>).ClipboardItem
    await dom.interact(() => fire(copyButton(2), 'click'))
    expect(await written[0]).toBe(runMarkdown(second, { replyChars: null }))
    expect(toggles).toEqual([])
  })

  it('shows on hover and keyboard focus, and always on an open row', async () => {
    await table(new Set([first.id]))
    const holder = (seq: number) => copyButton(seq).parentElement as Element
    expect(holder(1).getAttribute('class')).toContain('opacity-100')
    expect(holder(2).getAttribute('class')).toContain('opacity-0')
    expect(holder(2).getAttribute('class')).toContain('group-hover:opacity-100')
    expect(holder(2).getAttribute('class')).toContain(
      'group-focus-within:opacity-100',
    )
    expect(copyButton(2).getAttribute('aria-label')).toContain(
      'Copy run 2 as Markdown',
    )
  })

  it('copies a row that was open and closed again from the run as it is now, not the cached detail', async () => {
    await table(new Set([second.id]))
    const { clipboard, written } = fakeClipboard()
    Object.assign(navigator, { clipboard })
    delete (globalThis as Record<string, unknown>).ClipboardItem
    // The run changes after its row closed: the row's cached detail no longer matches it.
    const later = {
      ...second,
      reply: { ...second.reply, text: 'finished later' },
    }
    details.set(second.id, later)
    await dom.render(<Table openIds={new Set()} />)
    await dom.interact(() => fire(copyButton(2), 'click'))
    expect(await written[0]).toContain('finished later')
  })

  it('a second copy keeps "Copied" for another 1.5 s', async () => {
    await table()
    const { clipboard } = fakeClipboard()
    Object.assign(navigator, { clipboard })
    delete (globalThis as Record<string, unknown>).ClipboardItem
    const status = () =>
      dom.container.querySelector('[role="status"]')?.textContent ?? ''
    await dom.interact(() => fire(copyButton(2), 'click'))
    expect(status()).toBe('Copied')
    await Bun.sleep(1_000)
    await dom.interact(() => fire(copyButton(2), 'click'))
    await Bun.sleep(1_000)
    await dom.settle()
    // 2 s after the first copy, 1 s after the second: still shown.
    expect(status()).toBe('Copied')
    await Bun.sleep(700)
    await dom.settle()
    expect(status()).toBe('')
  })
})
