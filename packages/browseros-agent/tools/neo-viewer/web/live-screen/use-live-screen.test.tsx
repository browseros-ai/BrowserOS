/**
 * The live screen's controller against a viewer whose status check stops answering, as it does
 * when every connection the browser allows to the viewer is busy. Timers and the clock are Bun's
 * fakes, so seconds pass at once.
 */

import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { act, StrictMode } from 'react'
import type { LiveStatus } from '../../shared/live-screen'
import { installDom, type TestDom } from '../test-support/dom'
import { FOLLOW_STALE_MS, modeLine } from './live-screen-model'
import { startStatusPoll, useLiveScreen } from './use-live-screen'

const UP: LiveStatus = {
  host: { state: 'up' },
  agent: {
    tab: { tabId: 12, url: 'https://two.test/', title: 'Two' },
    taskId: null,
  },
  takeover: null,
}

/** Lets resolved promises run their callbacks; the fake clock does not move. */
async function microtasks(): Promise<void> {
  for (let i = 0; i < 10; i++) await Promise.resolve()
}

beforeEach(() => {
  jest.useFakeTimers()
})

afterEach(() => {
  jest.useRealTimers()
})

test('the status poll keeps one read in flight: a read that hangs is the only one until it answers', async () => {
  const answers: Array<(status: LiveStatus) => void> = []
  const delivered: LiveStatus[] = []
  const stop = startStatusPoll(
    () => new Promise((resolve) => answers.push(resolve)),
    (status) => delivered.push(status),
    10,
  )
  jest.advanceTimersByTime(100)
  expect(answers).toHaveLength(1)
  answers[0]?.(UP)
  await microtasks()
  expect(delivered).toEqual([UP])
  jest.advanceTimersByTime(10)
  expect(answers).toHaveLength(2)

  // Stopped while a read is pending: its answer is not delivered, and no read follows.
  stop()
  answers[1]?.(UP)
  await microtasks()
  jest.advanceTimersByTime(100)
  expect(delivered).toEqual([UP])
  expect(answers).toHaveLength(2)
})

test('a status read that never answers is aborted at its 5 s deadline, delivers nothing, and the next tick reads again', async () => {
  const reads: Array<{
    signal: AbortSignal
    answer: (status: LiveStatus) => void
  }> = []
  const delivered: LiveStatus[] = []
  const stop = startStatusPoll(
    (signal) => new Promise((answer) => reads.push({ signal, answer })),
    (status) => delivered.push(status),
    1_500,
  )
  jest.advanceTimersByTime(FOLLOW_STALE_MS - 1)
  expect(reads.map((read) => read.signal.aborted)).toEqual([false])
  jest.advanceTimersByTime(1)
  expect(reads.map((read) => read.signal.aborted)).toEqual([true])

  // The tick at 6 s reads again, with the stuck read still unsettled.
  jest.advanceTimersByTime(1_000)
  expect(reads.map((read) => read.signal.aborted)).toEqual([true, false])

  // The stuck read settles late: it delivers nothing, and the read after it is still the only one.
  reads[0]?.answer({
    host: { state: 'unreachable', message: 'The viewer did not answer' },
    agent: null,
    takeover: null,
  })
  await microtasks()
  jest.advanceTimersByTime(1_500)
  expect(reads).toHaveLength(2)
  reads[1]?.answer(UP)
  await microtasks()
  expect(delivered).toEqual([UP])
  stop()
})

test('under StrictMode the panel keeps one status read pending, and unmounting aborts it', async () => {
  const dom: TestDom = await installDom()
  // Each status read waits for a connection until it is aborted, as a fetch does.
  const reads: Array<AbortSignal | undefined> = []
  globalThis.fetch = ((_input: string, init?: RequestInit) => {
    const signal = init?.signal ?? undefined
    reads.push(signal)
    return new Promise((_resolve, reject) =>
      signal?.addEventListener('abort', () => reject(signal.reason)),
    )
  }) as unknown as typeof fetch
  const pending = () => reads.filter((signal) => !signal?.aborted).length
  function FollowLine() {
    return <p>{modeLine(useLiveScreen().state)}</p>
  }
  const { createRoot } = await import('react-dom/client')
  const root = createRoot(document.createElement('div'))
  await act(async () => {
    root.render(
      <StrictMode>
        <FollowLine />
      </StrictMode>,
    )
    await microtasks()
  })
  // StrictMode started the poll, stopped it and started it again.
  expect(reads).toHaveLength(2)
  expect(pending()).toBe(1)

  await act(async () => root.unmount())
  expect(pending()).toBe(0)
  await dom.restore()
})

test('the follow line says it is behind once no status has confirmed the agent’s tab for 5 s, and catches up when one does', async () => {
  const dom: TestDom = await installDom()
  let reads = 0
  const waiting: Array<(status: LiveStatus) => void> = []
  // The first status answers; every later one waits, as behind six open event streams.
  globalThis.fetch = (async () => {
    reads += 1
    const status =
      reads === 1
        ? UP
        : await new Promise<LiveStatus>((resolve) => waiting.push(resolve))
    return { ok: true, json: async () => status } as Response
  }) as unknown as typeof fetch
  function FollowLine() {
    return <p>{modeLine(useLiveScreen().state)}</p>
  }
  const { createRoot } = await import('react-dom/client')
  const element = document.createElement('div')
  const root = createRoot(element)
  const shown = () => element.textContent ?? ''
  await act(async () => {
    root.render(<FollowLine />)
    await microtasks()
  })
  expect(shown()).toStartWith('Following the agent’s tab.')

  // Polls at 1.5 s and 3 s: the first waits, the second starts nothing.
  await act(async () => {
    jest.advanceTimersByTime(4_000)
    await microtasks()
  })
  expect(reads).toBe(2)
  expect(shown()).toStartWith('Following the agent’s tab.')

  await act(async () => {
    jest.advanceTimersByTime(1_100)
    await microtasks()
  })
  expect(shown()).toBe(
    'Following is behind: the agent’s tab has not been confirmed for over 5 s, so the screen may show an older tab. It catches up when a status check answers.',
  )

  await act(async () => {
    waiting[0]?.(UP)
    await microtasks()
  })
  expect(shown()).toStartWith('Following the agent’s tab.')

  await act(async () => root.unmount())
  await dom.restore()
})
