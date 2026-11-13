import { expect, test } from 'bun:test'
import type { hostFetch } from './hostApi'
import {
  applyViewerSettings,
  TRACE_VALUE_BYTES,
  VIEWER_SETTINGS,
} from './neo-settings'

/** A `hostFetch` that answers each call with the next of `answers`, recording what it was sent. */
function fakeHost(answers: Array<() => Response>) {
  const calls: Array<{ path: string; init: RequestInit }> = []
  const send: typeof hostFetch = async (path, init = {}) => {
    calls.push({ path, init })
    const answer = answers[Math.min(calls.length - 1, answers.length - 1)]
    if (!answer) throw new Error('no answer scripted')
    return answer()
  }
  return { send, calls }
}

const applied = () =>
  Response.json({
    settings: { trace: { valueBytes: TRACE_VALUE_BYTES } },
    defaults: { trace: { valueBytes: 0 } },
    unknown: [],
    unknownValues: [],
  })
const noSleep = async () => {}

test('the viewer PUTs trace.valueBytes at Neo’s reply limit, nothing else', async () => {
  const host = fakeHost([applied])
  expect(await applyViewerSettings({ fetch: host.send })).toBe('applied')
  expect(TRACE_VALUE_BYTES).toBe(256 * 1024)
  expect(host.calls).toHaveLength(1)
  const [call] = host.calls
  expect(call?.path).toBe('/v1/settings')
  expect(call?.init.method).toBe('PUT')
  expect(JSON.parse(String(call?.init.body))).toEqual(VIEWER_SETTINGS)
  expect(VIEWER_SETTINGS).toEqual({ trace: { valueBytes: 262144 } })
})

test('it waits for the host API to serve, then sets it', async () => {
  const notYet = () =>
    Response.json(
      { code: 'host_api_unreachable', message: 'starting' },
      { status: 502 },
    )
  const host = fakeHost([notYet, notYet, applied])
  expect(await applyViewerSettings({ fetch: host.send, sleep: noSleep })).toBe(
    'applied',
  )
  expect(host.calls).toHaveLength(3)
})

test('an image older than /v1/settings is reported, not a failure', async () => {
  const host = fakeHost([
    () =>
      Response.json(
        { code: 'not_found', message: 'no route matches this path' },
        { status: 404 },
      ),
  ])
  expect(await applyViewerSettings({ fetch: host.send })).toBe('unsupported')
})

test('a refusal fails the start with Neo’s message, and so does a host API that never serves', async () => {
  const refused = fakeHost([
    () =>
      Response.json(
        {
          code: 'invalid_request',
          message: '`trace.valueBytes` must be a whole number from 0 to 262144',
        },
        { status: 400 },
      ),
  ])
  await expect(applyViewerSettings({ fetch: refused.send })).rejects.toThrow(
    'trace.valueBytes',
  )
  const silent = fakeHost([
    () =>
      Response.json(
        { code: 'host_api_unreachable', message: 'nothing answers' },
        { status: 502 },
      ),
  ])
  await expect(
    applyViewerSettings({ fetch: silent.send, sleep: noSleep, waitMs: 0 }),
  ).rejects.toThrow('was not serving')
})

test('a host API that takes the request and never answers cannot hold the start past the wait', async () => {
  const hung: typeof hostFetch = (_path, init = {}) =>
    new Promise<Response>((_resolve, reject) => {
      init.signal?.addEventListener('abort', () => reject(init.signal?.reason))
    })
  const started = Date.now()
  await expect(
    applyViewerSettings({ fetch: hung, sleep: noSleep, waitMs: 50 }),
  ).rejects.toThrow('did not answer')
  expect(Date.now() - started).toBeLessThan(5000)
})
