import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  type LiveError,
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
import { type TraceOperation, taskLastTab } from './live-routes'

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

/** Neo with two tabs; task `t1`'s operations targeted tab 11, then tab 12, then no tab. */
async function neoWithTask(): Promise<FakeNeoHost> {
  const neo = startFakeNeoHost()
  cleanups.push(() => neo.stop())
  let state: 'active' | 'released' = 'active'
  neo.respond = (request: HostRequest) => {
    const path = request.path
    if (path === '/v1/tabs') return Response.json({ items: TABS })
    // The agent's own task id narrows the trace; without one the viewer reads all of it.
    if (
      path === '/v1/trace?taskId=t1&tail=200' ||
      path === '/v1/trace?tail=200'
    )
      return Response.json({
        runs: [],
        operations: [
          { id: 'r1.1', name: 'page.goto', tabId: 11, startedAt: 1 },
          { id: 'r1.2', name: 'page.click', tabId: 12, startedAt: 2 },
          { id: 'r1.3', name: 'neo.sleep', startedAt: 3 },
        ],
        truncated: false,
      })
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

async function status(
  viewer: RunningViewer,
  task?: string,
): Promise<LiveStatus> {
  const query = task ? `?task=${task}` : ''
  return (await (
    await fetch(`${viewer.url}${liveRoutes.status}${query}`)
  ).json()) as LiveStatus
}

test('status names the tab the latest operation targeted, narrowed to an agent’s task id when given', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)
  expect(await status(viewer, 't1')).toEqual({
    host: { state: 'up' },
    tab: { tabId: 12, url: 'https://two.test/', title: 'Two' },
    takeover: null,
  })
  // Without a task id: the tab the latest operation used, across every run.
  expect((await status(viewer)).tab?.tabId).toBe(12)
  expect((await status(viewer, 'never-ran')).tab).toBeNull()
  expect(
    neo.requests.every((r) => r.authorization === `Bearer ${FAKE_TOKEN}`),
  ).toBe(true)
})

test('status says plainly when Neo is not there', async () => {
  const viewer = await viewerWith(unreachableHost())
  const answer = await status(viewer, 't1')
  expect(answer.host.state).toBe('unreachable')
  expect(answer.tab).toBeNull()
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
  expect((await status(viewer, 't1')).host).toEqual({
    state: 'unreachable',
    message: 'nothing answered',
  })
})

test('take over locks the task’s tab, status reports it held, give back releases it', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)

  const taken = await post(viewer, liveRoutes.takeovers, {
    task: 't1',
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

  // A reloaded page learns the takeover from status, even when it shows no task or another one.
  expect((await status(viewer, 't1')).takeover?.id).toBe('tk_1')
  expect((await status(viewer)).takeover?.id).toBe('tk_1')

  const given = await post(viewer, liveRoutes.release('tk_1'), {})
  expect(given.status).toBe(200)
  expect(((await given.json()) as TakeoverResponse).takeover.state).toBe(
    'released',
  )
  expect(JSON.parse(neo.requests.at(-1)?.body ?? '')).toEqual({
    reason: 'done',
  })
  expect((await status(viewer, 't1')).takeover).toBeNull()
})

test('a takeover Neo ended on its own is no longer reported held', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)
  await post(viewer, liveRoutes.takeovers, { task: 't1' })
  const answer = neo.respond
  neo.respond = (request) =>
    request.path === '/v1/takeovers/tk_1'
      ? Response.json({ ...takeover('released'), releaseReason: 'expired' })
      : answer(request)
  expect((await status(viewer, 't1')).takeover).toBeNull()
})

test('stopping the viewer gives back the takeovers it holds', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)
  await post(viewer, liveRoutes.takeovers, { task: 't1' })
  await viewer.stop()
  expect(
    neo.requests.some((r) => r.path === '/v1/takeovers/tk_1/release'),
  ).toBe(true)
})

test('take over says why it cannot, with Neo’s own error when Neo refused', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)

  const noTab = await post(viewer, liveRoutes.takeovers, { task: 'never-ran' })
  expect(noTab.status).toBe(409)
  expect(((await noTab.json()) as LiveError).error.code).toBe('no_tab')

  for (const body of [
    { task: '' },
    { task: 7 },
    { task: 't1', viewer: { width: 99, height: 1000 } },
    { task: 't1', viewer: { width: 1600.5, height: 1000 } },
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
  const refused = await post(viewer, liveRoutes.takeovers, { task: 't1' })
  expect(refused.status).toBe(404)
  expect(await refused.json()).toEqual({
    error: { code: 'tab_not_found', message: 'no open tab has this id' },
  })
})

test('take over and give back refuse another origin before reaching Neo', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)
  for (const origin of ['http://evil.example', 'null']) {
    expect(
      (await post(viewer, liveRoutes.takeovers, { task: 't1' }, origin)).status,
    ).toBe(403)
    expect(
      (await post(viewer, liveRoutes.release('tk_1'), {}, origin)).status,
    ).toBe(403)
  }
  const noOrigin = await fetch(`${viewer.url}${liveRoutes.takeovers}`, {
    method: 'POST',
    body: JSON.stringify({ task: 't1' }),
  })
  expect(noOrigin.status).toBe(403)
  const wrongHost = await fetch(`${viewer.url}${liveRoutes.status}`, {
    headers: { host: 'rebound.example' },
  })
  expect(wrongHost.status).toBe(403)
  expect(neo.requests).toHaveLength(0)
})

test('a takeover still starting when the viewer stops is given back too', async () => {
  const neo = await neoWithTask()
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
  const taking = post(viewer, liveRoutes.takeovers, { task: 't1' })
  await arrived
  await viewer.stop()
  expect((await taking).status).toBe(503)
  expect(
    neo.requests.some((r) => r.path === '/v1/takeovers/tk_1/release'),
  ).toBe(true)
  expect(
    (await post(viewer, liveRoutes.takeovers, { task: 't1' }).catch(() => null))
      ?.status ?? 'closed',
  ).not.toBe(200)
})

test('the task’s tab follows Neo’s rule: subscriptions skipped, a new page counts at its end', () => {
  const op = (
    id: string,
    name: string,
    startedAt: number,
    tabs: { tabId?: number; page?: number },
  ): TraceOperation => ({
    id,
    name,
    startedAt,
    ...(tabs.tabId === undefined ? {} : { tabId: tabs.tabId }),
    ...(tabs.page === undefined ? {} : { page: { tabId: tabs.page } }),
  })
  // An events.listen on another tab after the last act does not move the task.
  expect(
    taskLastTab([
      op('r1.1', 'page.goto', 1, { tabId: 11 }),
      op('r1.2', 'events.listen', 2, { tabId: 12 }),
      op('r1.3', 'events.next', 3, { tabId: 12 }),
    ]),
  ).toBe(11)
  // context.newPage has no target; its new tab arrives as the page it ended on.
  expect(
    taskLastTab([
      op('r1.1', 'page.goto', 1, { tabId: 11 }),
      op('r1.2', 'context.newPage', 2, { page: 13 }),
    ]),
  ).toBe(13)
  // The latest by start wins, whatever order the trace lists them in; then run, then seq.
  expect(
    taskLastTab([
      op('r2.1', 'page.click', 5, { tabId: 12 }),
      op('r1.9', 'page.click', 9, { tabId: 11 }),
      op('r1.10', 'page.fill', 9, { tabId: 14 }),
    ]),
  ).toBe(14)
  expect(taskLastTab([op('r1.1', 'neo.sleep', 1, {})])).toBeNull()
})

test('without a task id, take over takes the tab the latest operation used', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)
  const taken = await post(viewer, liveRoutes.takeovers, {})
  expect(taken.status).toBe(200)
  expect(
    neo.requests.some((request) => request.path === '/v1/trace?tail=200'),
  ).toBe(true)
  const body =
    neo.requests.find((request) => request.path === '/v1/takeovers')?.body ?? ''
  expect(JSON.parse(body)).toMatchObject({ tabId: 12 })
})
