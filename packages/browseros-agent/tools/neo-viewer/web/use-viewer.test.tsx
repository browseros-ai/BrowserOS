import { describe, expect, it } from 'bun:test'
import {
  apiRoutes,
  type RunDetail,
  type RunsResponse,
  type TelemetryHealth,
} from '../shared/api'
import { installDom } from './test-support/dom'
import { runDetail, runSummary } from './test-support/runs'
import { FakeOrigin } from './test-support/tabs'
import { health } from './test-support/telemetry'
import { keepNewer, useRunDetail, useViewer } from './use-viewer'

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
      snapshots[1]?.({
        runs: [runSummary(1), runSummary(2)],
        version: 2,
        health: null,
      }),
    )
    await dom.interact(() =>
      stream?.send({ version: 3, type: 'run', run: runSummary(3) }),
    )
    expect(dom.container.textContent).toBe('1 2 3')
    await dom.interact(() =>
      snapshots[0]?.({ runs: [runSummary(1)], version: 1, health: null }),
    )
    expect(dom.container.textContent).toBe('1 2 3')
    uninstall()
    await dom.restore()
  })
})

describe("Neo's health", () => {
  const at = (rev: number, tabsOpen = rev) => health({ rev, tabsOpen })

  it('keeps the highest rev: equal or lower is an older state, null says nothing new', () => {
    expect(keepNewer(null, at(2))).toEqual(at(2))
    expect(keepNewer(at(2), at(3))).toEqual(at(3))
    expect(keepNewer(at(3), at(2))).toEqual(at(3))
    expect(keepNewer(at(3), at(3, 99))).toEqual(at(3))
    expect(keepNewer(at(3), null)).toEqual(at(3))
    expect(keepNewer(null, null)).toBeNull()
  })

  it('takes health from snapshots and events, never rolls back, and never touches the run list', async () => {
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
    function Health() {
      const { runs, health: shown } = useViewer()
      return (
        <p>
          runs {runs.map((run) => run.seq).join(' ')} · tabs{' '}
          {shown ? shown.tabsOpen : 'none'}
        </p>
      )
    }
    await dom.render(<Health />)
    const [stream] = origin.openStreams()
    await dom.interact(() => stream?.open())
    // Health heard on the stream before the opening's snapshot answers.
    await dom.interact(() => stream?.send({ type: 'health', health: at(5) }))
    expect(dom.container.textContent).toBe('runs  · tabs 5')
    // The snapshot was read earlier (rev 4): its runs apply, its health does not.
    await dom.interact(() =>
      snapshots[0]?.({ runs: [runSummary(1)], version: 1, health: at(4) }),
    )
    expect(dom.container.textContent).toBe('runs 1 · tabs 5')
    await dom.interact(() => stream?.send({ type: 'health', health: at(6) }))
    await dom.interact(() =>
      stream?.send({ version: 2, type: 'run', run: runSummary(2) }),
    )
    expect(dom.container.textContent).toBe('runs 1 2 · tabs 6')
    uninstall()
    await dom.restore()
  })

  it('keeps what it shows across a reconnect, and follows a viewer restarted behind the stream', async () => {
    const dom = await installDom()
    const origin = new FakeOrigin()
    const uninstall = origin.tab().install()
    const answers: Array<TelemetryHealth | null> = [at(40), null]
    const pending: Array<() => void> = []
    globalThis.fetch = (async (input: string | URL) => {
      if (String(input) !== apiRoutes.runs) return Response.json({})
      const health = answers.shift() ?? null
      await new Promise<void>((resolve) => pending.push(resolve))
      return Response.json({ log: null, runs: [], version: 0, health })
    }) as typeof fetch
    function Health() {
      const { health: shown } = useViewer()
      return <p>{shown ? `rev ${shown.rev}` : 'none'}</p>
    }
    await dom.render(<Health />)
    const [stream] = origin.openStreams()
    await dom.interact(() => stream?.open())
    await dom.interact(() => pending.shift()?.())
    expect(dom.container.textContent).toBe('rev 40')
    // The viewer restarts: the stream reopens, and until the new opening says otherwise the
    // health shown stays (no blank on a reconnect).
    await dom.interact(() => {
      stream?.drop()
      stream?.open()
    })
    expect(dom.container.textContent).toBe('rev 40')
    // The new process counts from 1 again; its health still shows.
    await dom.interact(() => stream?.send({ type: 'health', health: at(1) }))
    expect(dom.container.textContent).toBe('rev 1')
    await dom.interact(() => pending.shift()?.())
    expect(dom.container.textContent).toBe('rev 1')
    uninstall()
    await dom.restore()
  })

  it('shows none when a reopened stream and its snapshot carry none (a restarted viewer without telemetry)', async () => {
    const dom = await installDom()
    const origin = new FakeOrigin()
    const uninstall = origin.tab().install()
    const answers: Array<TelemetryHealth | null> = [at(7), null]
    globalThis.fetch = (async (input: string | URL) =>
      String(input) === apiRoutes.runs
        ? Response.json({
            log: null,
            runs: [],
            version: 0,
            health: answers.shift() ?? null,
          })
        : Response.json({})) as typeof fetch
    function Health() {
      const { health: shown } = useViewer()
      return <p>{shown ? `rev ${shown.rev}` : 'none'}</p>
    }
    await dom.render(<Health />)
    const [stream] = origin.openStreams()
    await dom.interact(() => stream?.open())
    expect(dom.container.textContent).toBe('rev 7')
    await dom.interact(() => {
      stream?.drop()
      stream?.open()
    })
    expect(dom.container.textContent).toBe('none')
    uninstall()
    await dom.restore()
  })
})
