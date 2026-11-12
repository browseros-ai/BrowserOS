import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import type { RunSummary } from '../../shared/api'
import { runSummary } from '../test-support/runs'
import { TaskChips, taskHue } from './RunRow'

const run = (overrides: Partial<RunSummary>): RunSummary => ({
  ...runSummary(1),
  ...overrides,
})
const html = (summary: RunSummary) =>
  renderToStaticMarkup(<TaskChips run={summary} />)
const text = (markup: string) =>
  markup
    .replace(/<[^>]+>/g, ' ')
    .replace(/\s+/g, ' ')
    .trim()

describe('the task chips on a run row', () => {
  it('show nothing for a run with no task', () => {
    expect(html(run({}))).toBe('')
  })

  it('show one task as one chip, in its own stable hue', () => {
    const markup = html(run({ tasks: [{ id: 'shop-4b80', name: 'shop' }] }))
    expect(text(markup)).toBe('shop-4b80')
    expect(markup).toContain('title="task shop-4b80 (shop)"')
    const { background, color } = taskHue('shop-4b80')
    expect(markup).toContain(`background:${background};color:${color}`)
    // The same id, the same hue on every row; another id, another hue.
    expect(taskHue('shop-4b80')).toEqual(taskHue('shop-4b80'))
    expect(taskHue('t-91c2')).not.toEqual(taskHue('shop-4b80'))
  })

  it('stay compact, open or closed: the first and +N, all of them in the title', () => {
    const summary = run({
      tasks: [
        { id: 'shop-4b80', name: 'shop' },
        { id: 't-91c2', name: null },
      ],
      agentTaskId: 'host-1',
    })
    const closed = html(summary)
    expect(text(closed)).toBe('shop-4b80 +2')
    expect(closed).toContain(
      'title="task shop-4b80 (shop), task t-91c2, host task host-1"',
    )
  })

  it("show a host task (the request's _meta taskId) with a small host mark", () => {
    const markup = html(run({ agentTaskId: 'host-1' }))
    expect(text(markup)).toBe('host host-1')
    expect(markup).toContain('title="host task host-1"')
    // The script picked the host's task too: still one chip, still marked host.
    const picked = html(
      run({ agentTaskId: 'host-1', tasks: [{ id: 'host-1', name: null }] }),
    )
    expect(text(picked)).toBe('host host-1')
    expect(picked).toContain('title="host task host-1"')
  })
})
