import { describe, expect, it } from 'bun:test'
import type { RunDetail } from '../shared/api'
import { installDom } from './test-support/dom'
import { runDetail, runSummary } from './test-support/runs'
import { useRunDetail } from './use-viewer'

describe('a run read once', () => {
  it('starts a second view from what its block read, and keeps it when a refetch fails', async () => {
    const run = runSummary(731, { reply: '"read"' })
    const details = new Map<string, RunDetail>([
      [run.id, runDetail(run, '"read"')],
    ])
    const dom = await installDom(details)
    function Shown({ name }: { name: string }) {
      const detail = useRunDetail(run, true)
      return (
        <p>
          {name}: {detail ? detail.reply.text : 'Loading…'}
        </p>
      )
    }
    await dom.render(<Shown name="block" />)
    expect(dom.container.textContent).toBe('block: "read"')
    // The panel opens on the same run, and its own read fails.
    details.delete(run.id)
    await dom.render(<Shown name="panel" />)
    expect(dom.container.textContent).toBe('panel: "read"')
    dom.restore()
  })
})
