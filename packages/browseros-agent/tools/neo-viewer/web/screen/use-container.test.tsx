import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import type { ContainerInfo, ViewerInfo } from '../../shared/api'
import { installDom, type TestDom } from '../test-support/dom'
import { useContainer } from './use-container'

const RUNNING: ContainerInfo = {
  name: 'neo-viewer-sdk',
  image: 'neo-sdk:x',
  up: true,
}

const INFO: ViewerInfo = {
  mcpUrl: 'http://127.0.0.1:7310/mcp',
  upstreamUrl: 'http://127.0.0.1:39010/mcp',
  dataDir: '/tmp/viewer',
  startedAt: '2026-11-11T10:00:00.000Z',
  container: RUNNING,
}

function Line({ info }: { info: ViewerInfo | null }) {
  const container = useContainer(info, 10)
  return <p>{container ? (container.up ? 'Running' : 'Stopped') : 'none'}</p>
}

describe('the container line, kept current', () => {
  let dom: TestDom
  // What `docker inspect` says right now; each test changes it.
  let inspected: ContainerInfo | null
  let asked: number
  beforeEach(async () => {
    dom = await installDom()
    inspected = RUNNING
    asked = 0
    globalThis.fetch = (async () => {
      asked++
      return Response.json({ ...INFO, container: inspected })
    }) as unknown as typeof fetch
  })
  afterEach(() => dom.restore())

  const wait = (ms: number) =>
    dom
      .interact(() => {})
      .then(async () => {
        await new Promise((resolve) => setTimeout(resolve, ms))
        await dom.settle()
      })

  it('follows the container from running to stopped and back without a reload', async () => {
    await dom.render(<Line info={INFO} />)
    expect(dom.container.textContent).toBe('Running')
    inspected = { ...RUNNING, up: false }
    await wait(30)
    expect(dom.container.textContent).toBe('Stopped')
    inspected = RUNNING
    await wait(30)
    expect(dom.container.textContent).toBe('Running')
  })

  it('asks nothing when the viewer forwards to a Neo it did not start', async () => {
    await dom.render(<Line info={{ ...INFO, container: null }} />)
    await wait(40)
    expect(dom.container.textContent).toBe('none')
    expect(asked).toBe(0)
  })
})
