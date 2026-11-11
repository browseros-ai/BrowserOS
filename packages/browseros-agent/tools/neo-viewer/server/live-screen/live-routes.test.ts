import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  type LiveStatus,
  type LiveTakeover,
  liveRoutes,
  type TakeoverResponse,
} from '../../shared/live-screen'
import {
  FAKE_TOKEN,
  type FakeNeoHost,
  type HostRequest,
  startFakeNeoHost,
  unreachableHost,
} from '../test-support/fake-neo-host'
import { type RunningViewer, startViewer } from '../viewer-server'
import type { LiveHost } from './live-host'
import { agentFocus, type TraceTail } from './live-routes'

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

async function viewerWith(liveHost: LiveHost): Promise<RunningViewer> {
  const dataDir = await mkdtemp(join(tmpdir(), 'neo-viewer-live-'))
  cleanups.push(() => rm(dataDir, { recursive: true, force: true }))
  const viewer = await startViewer(
    { port: 0, upstream: new URL('http://127.0.0.1:9/mcp'), dataDir },
    { liveHost },
  )
  cleanups.push(() => viewer.stop())
  return viewer
}

const TABS = [
  {
    tabId: 11,
    targetId: 'A',
    url: 'https://one.test/',
    title: 'One',
    taskIds: [],
  },
  {
    tabId: 12,
    targetId: 'B',
    url: 'https://two.test/',
    title: 'Two',
    taskIds: ['t1'],
  },
]

function takeover(state: 'active' | 'released'): LiveTakeover {
  return {
    id: 'tk_1',
    tabId: 12,
    state,
    display: '/v1/display?mode=control&release=tk_1',
    lockId: 'lk_1',
    createdAt: 1,
    expiresAt: 1_200_001,
    ...(state === 'released' ? { releaseReason: 'done', releasedAt: 2 } : {}),
  } as LiveTakeover
}

/** Neo's latest operations: one run without a task id, on tab 11, then tab 12, then no tab. */
const UNTASKED: TraceTail = {
  runs: [{ id: 'r1', startedAt: 1 }],
  operations: [
    { id: 'r1.1', runId: 'r1', name: 'page.goto', tabId: 11 },
    { id: 'r1.2', runId: 'r1', name: 'page.click', tabId: 12 },
    { id: 'r1.3', runId: 'r1', name: 'neo.sleep' },
  ],
}

/** Neo with two tabs, whose trace is `trace` (by default the agent's last act was on tab 12). */
async function neoWithAgent(
  trace: { current: TraceTail } = { current: UNTASKED },
): Promise<FakeNeoHost> {
  const neo = startFakeNeoHost()
  cleanups.push(() => neo.stop())
  let state: 'active' | 'released' = 'active'
  neo.respond = (request: HostRequest) => {
    const path = request.path
    if (path === '/v1/tabs') return Response.json({ items: TABS })
    if (path === '/v1/trace?tail=100')
      return Response.json({ ...trace.current, truncated: false })
    if (path === '/v1/takeovers' && request.method === 'POST') {
      state = 'active'
      return Response.json(takeover(state))
    }
    if (path === '/v1/takeovers/tk_1') return Response.json(takeover(state))
    if (path === '/v1/takeovers/tk_1/release') {
      state = 'released'
      return Response.json(takeover(state))
    }
    return undefined
  }
  return neo
}

function post(
  viewer: RunningViewer,
  path: string,
  body: unknown,
  origin = viewer.url,
) {
  return fetch(`${viewer.url}${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', origin },
    body: JSON.stringify(body),
  })
}

async function status(viewer: RunningViewer): Promise<LiveStatus> {
  return (await (
    await fetch(`${viewer.url}${liveRoutes.status}`)
  ).json()) as LiveStatus
}

const TAB_ONE = { tabId: 11, url: 'https://one.test/', title: 'One' }
const TAB_TWO = { tabId: 12, url: 'https://two.test/', title: 'Two' }

test('an agent that names no task is followed by the tab of its latest run’s last operation', async () => {
  const neo = await neoWithAgent()
  const viewer = await viewerWith(neo.host)
  expect(await status(viewer)).toEqual({
    host: { state: 'up' },
    agent: { tab: TAB_TWO, taskId: null },
    takeover: null,
  })
  expect(
    neo.requests.every((r) => r.authorization === `Bearer ${FAKE_TOKEN}`),
  ).toBe(true)
})

test('a run that named its own task is followed by that task', async () => {
  const trace = { current: UNTASKED }
  const neo = await neoWithAgent(trace)
  const viewer = await viewerWith(neo.host)
  trace.current = {
    runs: [...UNTASKED.runs, { id: 'r2', taskId: 'zuzu-7', startedAt: 2 }],
    operations: [
      ...UNTASKED.operations,
      { id: 'r2.1', runId: 'r2', name: 'page.goto', tabId: 11 },
    ],
  }
  expect((await status(viewer)).agent).toEqual({
    tab: TAB_ONE,
    taskId: 'zuzu-7',
  })
  // Before the agent acts on a tab, and once its tab has closed, there is nothing to follow.
  trace.current = { runs: [], operations: [] }
  expect((await status(viewer)).agent).toBeNull()
  trace.current = {
    runs: [{ id: 'r3', startedAt: 3 }],
    operations: [{ id: 'r3.1', runId: 'r3', name: 'page.goto', tabId: 99 }],
  }
  expect((await status(viewer)).agent).toBeNull()
})

test('status says plainly when Neo is not there', async () => {
  const viewer = await viewerWith(unreachableHost())
  const answer = await status(viewer)
  expect(answer.host.state).toBe('unreachable')
  expect(answer.agent).toBeNull()
  expect(JSON.stringify(answer)).not.toContain(FAKE_TOKEN)
})

test('status reads the host API client’s “no Neo” answer as unreachable', async () => {
  const viewer = await viewerWith({
    fetch: async () =>
      Response.json(
        { code: 'host_api_unreachable', message: 'nothing answered' },
        { status: 502 },
      ),
    socket: (path) => unreachableHost().socket(path),
  })
  expect((await status(viewer)).host).toEqual({
    state: 'unreachable',
    message: 'nothing answered',
  })
})

test('take over locks the agent’s tab, status reports it held, give back releases it', async () => {
  const neo = await neoWithAgent()
  const viewer = await viewerWith(neo.host)

  const taken = await post(viewer, liveRoutes.takeovers, {
    tabId: 12,
    viewer: { width: 1600, height: 1000 },
  })
  expect(taken.status).toBe(200)
  const { takeover: held } = (await taken.json()) as TakeoverResponse
  expect(held).toMatchObject({ id: 'tk_1', tabId: 12, state: 'active' })
  const start = neo.requests.find((r) => r.path === '/v1/takeovers')
  expect(start?.method).toBe('POST')
  // Neo's own screen size as the viewer: its fit brings the tab to the front.
  expect(JSON.parse(start?.body ?? '')).toEqual({
    tabId: 12,
    viewer: { width: 1600, height: 1000 },
  })

  // A reloaded page learns the takeover from status.
  expect((await status(viewer)).takeover?.id).toBe('tk_1')

  const given = await post(viewer, liveRoutes.release('tk_1'), {})
  expect(given.status).toBe(200)
  expect(((await given.json()) as TakeoverResponse).takeover.state).toBe(
    'released',
  )
  expect(JSON.parse(neo.requests.at(-1)?.body ?? '')).toEqual({
    reason: 'done',
  })
  expect((await status(viewer)).takeover).toBeNull()
})

test('a takeover Neo ended on its own is no longer reported held', async () => {
  const neo = await neoWithAgent()
  const viewer = await viewerWith(neo.host)
  await post(viewer, liveRoutes.takeovers, { tabId: 12 })
  const answer = neo.respond
  neo.respond = (request) =>
    request.path === '/v1/takeovers/tk_1'
      ? Response.json({ ...takeover('released'), releaseReason: 'expired' })
      : answer(request)
  expect((await status(viewer)).takeover).toBeNull()
})

test('stopping the viewer gives back the takeovers it holds', async () => {
  const neo = await neoWithAgent()
  const viewer = await viewerWith(neo.host)
  await post(viewer, liveRoutes.takeovers, { tabId: 12 })
  await viewer.stop()
  expect(
    neo.requests.some((r) => r.path === '/v1/takeovers/tk_1/release'),
  ).toBe(true)
})

test('take over says why it cannot, with Neo’s own error when Neo refused', async () => {
  const neo = await neoWithAgent()
  const viewer = await viewerWith(neo.host)

  for (const body of [
    { nope: true },
    { tabId: '12' },
    { tabId: -1 },
    { tabId: 1.5 },
    { tabId: 12, viewer: { width: 99, height: 1000 } },
    { tabId: 12, viewer: { width: 1600.5, height: 1000 } },
  ])
    expect((await post(viewer, liveRoutes.takeovers, body)).status).toBe(400)

  const answer = neo.respond
  neo.respond = (request) =>
    request.path === '/v1/takeovers'
      ? Response.json(
          { code: 'tab_not_found', message: 'no open tab has this id' },
          { status: 404 },
        )
      : answer(request)
  const refused = await post(viewer, liveRoutes.takeovers, { tabId: 12 })
  expect(refused.status).toBe(404)
  expect(await refused.json()).toEqual({
    error: { code: 'tab_not_found', message: 'no open tab has this id' },
  })
})

test('take over and give back refuse another origin before reaching Neo', async () => {
  const neo = await neoWithAgent()
  const viewer = await viewerWith(neo.host)
  for (const origin of ['http://evil.example', 'null']) {
    expect(
      (await post(viewer, liveRoutes.takeovers, { tabId: 12 }, origin)).status,
    ).toBe(403)
    expect(
      (await post(viewer, liveRoutes.release('tk_1'), {}, origin)).status,
    ).toBe(403)
  }
  const noOrigin = await fetch(`${viewer.url}${liveRoutes.takeovers}`, {
    method: 'POST',
    body: JSON.stringify({ tabId: 12 }),
  })
  expect(noOrigin.status).toBe(403)
  const wrongHost = await fetch(`${viewer.url}${liveRoutes.status}`, {
    headers: { host: 'rebound.example' },
  })
  expect(wrongHost.status).toBe(403)
  expect(neo.requests).toHaveLength(0)
})

test('a takeover still starting when the viewer stops is given back too', async () => {
  const neo = await neoWithAgent()
  const answer = neo.respond
  let startArrived = () => {}
  const arrived = new Promise<void>((resolve) => {
    startArrived = resolve
  })
  neo.respond = (request) => {
    if (request.path === '/v1/takeovers') startArrived()
    return answer(request)
  }
  // Neo's answer to the start reaches the viewer only after `stop` has begun.
  const realFetch = neo.host.fetch
  neo.host.fetch = async (path, init) => {
    const response = await realFetch(path, init)
    if (path === '/v1/takeovers') await Bun.sleep(300)
    return response
  }
  const viewer = await viewerWith(neo.host)
  const taking = post(viewer, liveRoutes.takeovers, { tabId: 12 })
  await arrived
  await viewer.stop()
  expect((await taking).status).toBe(503)
  expect(
    neo.requests.some((r) => r.path === '/v1/takeovers/tk_1/release'),
  ).toBe(true)
  expect(
    (await post(viewer, liveRoutes.takeovers, { tabId: 12 }).catch(() => null))
      ?.status ?? 'closed',
  ).not.toBe(200)
})

test('the agent’s tab: subscriptions act on no tab, a new page counts, a tabless run keeps the last', () => {
  const trace = (
    ...ops: Array<
      [id: string, name: string, tabs?: { tabId?: number; page?: number }]
    >
  ): TraceTail => {
    const runIds = [...new Set(ops.map(([id]) => id.split('.')[0] ?? ''))]
    return {
      runs: runIds.map((id, i) => ({
        id,
        startedAt: i + 1,
        ...(id.endsWith('t') ? { taskId: `task-${id}` } : {}),
      })),
      operations: ops.map(([id, name, tabs = {}]) => ({
        id,
        runId: id.split('.')[0] ?? '',
        name,
        ...(tabs.tabId === undefined ? {} : { tabId: tabs.tabId }),
        ...(tabs.page === undefined ? {} : { page: { tabId: tabs.page } }),
      })),
    }
  }
  // An events.listen on another tab after the last act does not move the agent.
  expect(
    agentFocus(
      trace(
        ['r1.1', 'page.goto', { tabId: 11 }],
        ['r1.2', 'events.listen', { tabId: 12 }],
        ['r1.3', 'events.next', { tabId: 12 }],
      ),
    ),
  ).toEqual({ tabId: 11, taskId: null })
  // context.newPage has no target; its new tab arrives as the page it ended on.
  expect(
    agentFocus(
      trace(
        ['r1.1', 'page.goto', { tabId: 11 }],
        ['r1.2', 'context.newPage', { page: 13 }],
      ),
    ),
  ).toEqual({ tabId: 13, taskId: null })
  // The most recent run wins, then its last operation; a run on no tab leaves the one before.
  expect(
    agentFocus(
      trace(
        ['r1.9', 'page.click', { tabId: 11 }],
        ['r2t.1', 'page.click', { tabId: 12 }],
        ['r2t.2', 'page.fill', { tabId: 14 }],
        ['r3.1', 'neo.sleep'],
      ),
    ),
  ).toEqual({ tabId: 14, taskId: 'task-r2t' })
  expect(agentFocus(trace(['r1.1', 'neo.sleep']))).toBeNull()
})
