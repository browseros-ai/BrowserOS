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
    if (path === '/v1/trace?taskId=t1&tail=100')
      return Response.json({
        runs: [],
        operations: [{ tabId: 11 }, { tabId: 12 }, { name: 'neo.sleep' }],
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

test('status names the tab the task’s latest operation targeted', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)
  expect(await status(viewer, 't1')).toEqual({
    host: { state: 'up' },
    tab: { tabId: 12, url: 'https://two.test/', title: 'Two' },
    takeover: null,
  })
  expect((await status(viewer)).tab).toBeNull()
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

test('take over locks the task’s tab, status reports it held, give back releases it', async () => {
  const neo = await neoWithTask()
  const viewer = await viewerWith(neo.host)

  const taken = await post(viewer, liveRoutes.takeovers, { task: 't1' })
  expect(taken.status).toBe(200)
  const { takeover: held } = (await taken.json()) as TakeoverResponse
  expect(held).toMatchObject({ id: 'tk_1', tabId: 12, state: 'active' })
  const start = neo.requests.find((r) => r.path === '/v1/takeovers')
  expect(start?.method).toBe('POST')
  expect(JSON.parse(start?.body ?? '')).toEqual({ tabId: 12 })

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

  const bad = await post(viewer, liveRoutes.takeovers, { nope: true })
  expect(bad.status).toBe(400)

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
