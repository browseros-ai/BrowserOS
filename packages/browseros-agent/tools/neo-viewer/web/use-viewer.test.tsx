import { describe, expect, it } from 'bun:test'
import { apiRoutes, type RunDetail, type RunsResponse } from '../shared/api'
import { installDom } from './test-support/dom'
import { runDetail, runSummary } from './test-support/runs'
import { FakeOrigin } from './test-support/tabs'
import { useRunDetail, useViewer } from './use-viewer'

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
    await dom.restore()
  })
})

describe('the event stream', () => {
  it('every view of the runs on one origin hears one stream: each lists its snapshot, then every run after it', async () => {
    const dom = await installDom()
    const origin = new FakeOrigin()
    const uninstall = origin.tab().install()
    globalThis.fetch = (async (input: string | URL) =>
      String(input) === apiRoutes.runs
        ? Response.json({ runs: [runSummary(1)], log: null, version: 1 })
        : Response.json({})) as typeof fetch
    // Each `useViewer` subscribes to the stream as a viewer tab does.
    function Runs({ name }: { name: string }) {
      const { runs } = useViewer()
      return (
        <p>
          {name}: {runs.map((run) => run.seq).join(' ')}
        </p>
      )
    }
    await dom.render(
      <>
        <Runs name="a" />
        <Runs name="b" />
      </>,
    )
    expect(origin.openStreams()).toHaveLength(1)
    const [stream] = origin.openStreams()
    await dom.interact(() => stream?.open())
    expect(dom.container.textContent).toBe('a: 1b: 1')
    await dom.interact(() =>
      stream?.send({ version: 2, type: 'run', run: runSummary(2) }),
    )
    expect(dom.container.textContent).toBe('a: 1 2b: 1 2')
    expect(origin.streams).toHaveLength(1)
    uninstall()
    await dom.restore()
  })

  it('a snapshot taken for an earlier opening that answers after a later one changes nothing', async () => {
    const dom = await installDom()
    const origin = new FakeOrigin()
    const uninstall = origin.tab().install()
    type Snapshot = Omit<RunsResponse, 'log'>
    const snapshots: Array<(answer: Snapshot) => void> = []
    globalThis.fetch = (async (input: string | URL) =>
      String(input) === apiRoutes.runs
        ? Response.json({
            log: null,
            ...(await new Promise<Snapshot>((resolve) =>
              snapshots.push(resolve),
            )),
          })
        : Response.json({})) as typeof fetch
    function Runs() {
      const { runs } = useViewer()
      return <p>{runs.map((run) => run.seq).join(' ')}</p>
    }
    await dom.render(<Runs />)
    const [stream] = origin.openStreams()
    await dom.interact(() => stream?.open())
    // The stream drops and reopens before the first snapshot answers.
    await dom.interact(() => {
      stream?.drop()
      stream?.open()
    })
    expect(snapshots).toHaveLength(2)
    await dom.interact(() =>
      snapshots[1]?.({ runs: [runSummary(1), runSummary(2)], version: 2 }),
    )
    await dom.interact(() =>
      stream?.send({ version: 3, type: 'run', run: runSummary(3) }),
    )
    expect(dom.container.textContent).toBe('1 2 3')
    await dom.interact(() =>
      snapshots[0]?.({ runs: [runSummary(1)], version: 1 }),
    )
    expect(dom.container.textContent).toBe('1 2 3')
    uninstall()
    await dom.restore()
  })
})
