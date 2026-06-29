/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { env } from '../../src/env'
import { setLocalServerUrl } from '../../src/local-server-url'
import {
  _resetReplayInjectionForTesting,
  ensureReplayRecorder,
} from '../../src/services/replay-injection'

interface RecordedCall {
  source: string
  runImmediately: boolean | undefined
}

interface FakeBrowserSessionOpts {
  recordCalls: RecordedCall[]
  shouldThrow?: boolean
}

function fakeBrowserSession(opts: FakeBrowserSessionOpts) {
  return {
    pages: {
      getSession: async () => {
        if (opts.shouldThrow) {
          throw new Error('CDP target detached')
        }
        return {
          session: {
            Page: {
              addScriptToEvaluateOnNewDocument: async (args: {
                source: string
                runImmediately?: boolean
              }) => {
                opts.recordCalls.push({
                  source: args.source,
                  runImmediately: args.runImmediately,
                })
              },
            },
          },
        }
      },
    },
    // biome-ignore lint/suspicious/noExplicitAny: stub for tests
  } as any
}

beforeEach(() => {
  _resetReplayInjectionForTesting()
  setLocalServerUrl('http://127.0.0.1:9200')
  // Recorder is off by default; the tests exercise the
  // happy-path injection so flip it on per case.
  env.replayEnabled = true
})

afterEach(() => {
  setLocalServerUrl(null)
  env.replayEnabled = false
})

describe('ensureReplayRecorder', () => {
  it('injects exactly once per pageId across multiple calls', async () => {
    const calls: RecordedCall[] = []
    const session = fakeBrowserSession({ recordCalls: calls })

    await ensureReplayRecorder({
      sessionId: 'sess-1',
      slug: 'agent',
      pageId: 42,
      session,
    })
    await ensureReplayRecorder({
      sessionId: 'sess-1',
      slug: 'agent',
      pageId: 42,
      session,
    })

    expect(calls).toHaveLength(1)
  })

  it('injects per pageId for different tabs in the same session', async () => {
    const calls: RecordedCall[] = []
    const session = fakeBrowserSession({ recordCalls: calls })

    await ensureReplayRecorder({
      sessionId: 'sess-1',
      slug: 'a',
      pageId: 1,
      session,
    })
    await ensureReplayRecorder({
      sessionId: 'sess-1',
      slug: 'a',
      pageId: 2,
      session,
    })

    expect(calls).toHaveLength(2)
  })

  it('does NOT use runImmediately (renderer stress / CDP drop mitigation)', async () => {
    const calls: RecordedCall[] = []
    const session = fakeBrowserSession({ recordCalls: calls })

    await ensureReplayRecorder({
      sessionId: 'sess-1',
      slug: 'a',
      pageId: 3,
      session,
    })

    expect(calls[0].runImmediately).toBeUndefined()
  })

  it('bakes sessionId + pageId + cockpit URL into the injected script', async () => {
    const calls: RecordedCall[] = []
    const session = fakeBrowserSession({ recordCalls: calls })

    await ensureReplayRecorder({
      sessionId: 'unique-session-id',
      slug: 'a',
      pageId: 77,
      session,
    })

    const src = calls[0].source
    expect(src).toContain('"unique-session-id"')
    expect(src).toContain('var tabPageId = 77')
    expect(src).toContain('"http://127.0.0.1:9200"')
  })

  it('logs and swallows CDP errors rather than throwing', async () => {
    const calls: RecordedCall[] = []
    const session = fakeBrowserSession({
      recordCalls: calls,
      shouldThrow: true,
    })

    await expect(
      ensureReplayRecorder({
        sessionId: 'sess-1',
        slug: 'a',
        pageId: 99,
        session,
      }),
    ).resolves.toBeUndefined()
    expect(calls).toHaveLength(0)
  })

  it('skips injection when sessionId is empty', async () => {
    const calls: RecordedCall[] = []
    const session = fakeBrowserSession({ recordCalls: calls })

    await ensureReplayRecorder({
      sessionId: '',
      slug: 'a',
      pageId: 11,
      session,
    })

    expect(calls).toHaveLength(0)
  })

  it('skips injection when the cockpit URL is not yet set', async () => {
    setLocalServerUrl(null)
    const calls: RecordedCall[] = []
    const session = fakeBrowserSession({ recordCalls: calls })

    await ensureReplayRecorder({
      sessionId: 'sess-1',
      slug: 'a',
      pageId: 12,
      session,
    })

    expect(calls).toHaveLength(0)
  })

  it('skips injection when CLAW_REPLAY_ENABLED is not set', async () => {
    env.replayEnabled = false
    const calls: RecordedCall[] = []
    const session = fakeBrowserSession({ recordCalls: calls })

    await ensureReplayRecorder({
      sessionId: 'sess-1',
      slug: 'a',
      pageId: 13,
      session,
    })

    expect(calls).toHaveLength(0)
  })
})
