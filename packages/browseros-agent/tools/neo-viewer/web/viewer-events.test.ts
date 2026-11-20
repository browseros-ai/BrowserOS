/**
 * Several viewer tabs on one fake origin (`test-support/tabs.ts`): what each tab's listener
 * hears, and how many event streams the browser holds open for them.
 */

import { expect, test } from 'bun:test'
import type { ViewerEvent } from '../shared/api'
import { runSummary } from './test-support/runs'
import { FakeOrigin, settle } from './test-support/tabs'
import { listenToViewerEvents, type StreamDeps } from './viewer-events'

/** Starts one subscription; `heard` is what its listener got: `opened`, or each event's version. */
function listen(deps: StreamDeps) {
  const heard: string[] = []
  const stop = listenToViewerEvents(
    {
      opened: () => heard.push('opened'),
      event: (event) =>
        heard.push('version' in event ? `v${event.version}` : event.type),
    },
    deps,
  )
  return { heard, stop }
}

function run(version: number): ViewerEvent {
  return { version, type: 'run', run: runSummary(version) }
}

test('one tab leads: it opens the one stream and hears it as the viewer always has', async () => {
  const origin = new FakeOrigin()
  const tab = listen(origin.tab().deps)
  await settle()
  expect(origin.openStreams().map((stream) => stream.url)).toEqual([
    '/api/events',
  ])
  expect(tab.heard).toEqual([])
  const [stream] = origin.openStreams()
  stream?.open()
  stream?.send(run(1))
  stream?.send(run(2))
  expect(tab.heard).toEqual(['opened', 'v1', 'v2'])

  tab.stop()
  expect(origin.openStreams()).toEqual([])
})

test('three tabs hear one stream: each takes one snapshot, then every event in order', async () => {
  const origin = new FakeOrigin()
  const tabs = [origin.tab(), origin.tab(), origin.tab()].map((tab) =>
    listen(tab.deps),
  )
  await settle()
  const [stream] = origin.openStreams()
  stream?.open()
  for (const version of [1, 2, 3]) stream?.send(run(version))
  await settle()
  for (const tab of tabs)
    expect(tab.heard).toEqual(['opened', 'v1', 'v2', 'v3'])
  expect(origin.streams).toHaveLength(1)
})

test('a tab that joins an open stream takes its own snapshot; the others take no other', async () => {
  const origin = new FakeOrigin()
  const [a, b] = [origin.tab(), origin.tab()].map((tab) => listen(tab.deps))
  await settle()
  const [stream] = origin.openStreams()
  stream?.open()
  stream?.send(run(1))
  await settle()
  const c = listen(origin.tab().deps)
  await settle()
  stream?.send(run(2))
  await settle()
  expect(a?.heard).toEqual(['opened', 'v1', 'v2'])
  expect(b?.heard).toEqual(['opened', 'v1', 'v2'])
  expect(c.heard).toEqual(['opened', 'v2'])
  expect(origin.streams).toHaveLength(1)
})

test('a hello that crosses the leader’s announcement of its opening: that tab takes one snapshot, not two', async () => {
  const origin = new FakeOrigin()
  const a = listen(origin.tab().deps)
  await settle()
  const [stream] = origin.openStreams()
  // b's hello is still on its way when the stream opens: the leader announces the opening to
  // every tab, then answers b's hello with the same opening.
  const b = listen(origin.tab().deps)
  stream?.open()
  await settle()
  stream?.send(run(1))
  await settle()
  expect(a.heard).toEqual(['opened', 'v1'])
  expect(b.heard).toEqual(['opened', 'v1'])
})

test('the leading tab closes: the next tab opens a new stream, and every tab starts again from it', async () => {
  const origin = new FakeOrigin()
  const tabs = [origin.tab(), origin.tab(), origin.tab()]
  const [a, b, c] = tabs.map((tab) => listen(tab.deps))
  await settle()
  const [first] = origin.openStreams()
  first?.open()
  first?.send(run(1))
  await settle()
  // The leader's relay of v2 is still on its way when its tab closes.
  tabs[0]?.hold()
  first?.send(run(2))
  tabs[0]?.close()
  await settle()
  expect(origin.openStreams().map((stream) => stream.tab)).toEqual([tabs[1]])
  const [second] = origin.openStreams()
  second?.open()
  await settle()
  tabs[0]?.release()
  await settle()
  second?.send(run(3))
  await settle()
  expect(a?.heard).toEqual(['opened', 'v1', 'v2'])
  // v2 came under the closed stream's opening, after the new one: the new snapshot holds it.
  expect(b?.heard).toEqual(['opened', 'v1', 'opened', 'v3'])
  expect(c?.heard).toEqual(['opened', 'v1', 'opened', 'v3'])
})

test('a tab stopped before its turn never opens a stream: the tab after it leads', async () => {
  const origin = new FakeOrigin()
  const tabs = [origin.tab(), origin.tab(), origin.tab()]
  const [, b, c] = tabs.map((tab) => listen(tab.deps))
  await settle()
  origin.openStreams()[0]?.open()
  await settle()
  b?.stop()
  tabs[0]?.close()
  await settle()
  expect(origin.openStreams().map((stream) => stream.tab)).toEqual([tabs[2]])
  const [second] = origin.openStreams()
  second?.open()
  second?.send(run(1))
  expect(b?.heard).toEqual(['opened'])
  expect(c?.heard).toEqual(['opened', 'opened', 'v1'])
})

test('mounted, unmounted and mounted again in one tab (React StrictMode): one stream, heard once', async () => {
  const origin = new FakeOrigin()
  const tab = origin.tab()
  const first = listen(tab.deps)
  first.stop()
  const second = listen(tab.deps)
  await settle()
  expect(origin.openStreams()).toHaveLength(1)
  origin.openStreams()[0]?.open()
  await settle()
  expect(first.heard).toEqual([])
  expect(second.heard).toEqual(['opened'])
})

test('the stream drops and reopens: every tab takes a fresh snapshot; a tab that joined meanwhile, one', async () => {
  const origin = new FakeOrigin()
  const [a, b] = [origin.tab(), origin.tab()].map((tab) => listen(tab.deps))
  await settle()
  const [stream] = origin.openStreams()
  stream?.open()
  stream?.send(run(1))
  stream?.drop()
  const c = listen(origin.tab().deps)
  await settle()
  expect(c.heard).toEqual([])
  stream?.open()
  stream?.send(run(2))
  await settle()
  expect(a?.heard).toEqual(['opened', 'v1', 'opened', 'v2'])
  expect(b?.heard).toEqual(['opened', 'v1', 'opened', 'v2'])
  expect(c.heard).toEqual(['opened', 'v2'])
})

test('without Web Locks or without BroadcastChannel, each tab opens its own stream, as before', async () => {
  for (const missing of ['locks', 'channel'] as const) {
    const origin = new FakeOrigin()
    const tabs = [origin.tab(), origin.tab()]
    const listeners = tabs.map((tab) =>
      listen({ ...tab.deps, [missing]: undefined }),
    )
    await settle()
    expect(origin.openStreams().map((stream) => stream.tab)).toEqual(tabs)
    origin.streams.forEach((stream, i) => {
      stream.open()
      stream.send(run(i + 1))
    })
    await settle()
    expect(listeners.map((tab) => tab.heard)).toEqual([
      ['opened', 'v1'],
      ['opened', 'v2'],
    ])
  }
})
