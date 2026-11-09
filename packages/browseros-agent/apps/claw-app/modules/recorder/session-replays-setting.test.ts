import { describe, expect, it } from 'bun:test'
import type { AgentSettings } from '@browseros/claw-api'
import { createSessionReplaysSetting } from './session-replays-setting'

const serverBaseUrl = 'http://127.0.0.1:9511'

function settingsServer(answer: () => Partial<AgentSettings> | Error) {
  const requests: string[] = []
  const fetch = async (input: Parameters<typeof globalThis.fetch>[0]) => {
    requests.push(String(input instanceof Request ? input.url : input))
    const body = answer()
    if (body instanceof Error) throw body
    return new Response(
      JSON.stringify({
        humanHelpEnabled: true,
        taskCleanupTipEnabled: true,
        sessionReplaysEnabled: true,
        ...body,
      }),
      { status: 200, headers: { 'content-type': 'application/json' } },
    )
  }
  return { fetch, requests }
}

describe('createSessionReplaysSetting', () => {
  it('tests that a new page does not record once the person turned replays off', async () => {
    const server = settingsServer(() => ({ sessionReplaysEnabled: false }))
    const setting = createSessionReplaysSetting({
      resolveServerBaseUrl: async () => serverBaseUrl,
      fetch: server.fetch,
    })

    expect(await setting.enabled()).toBe(false)
    expect(server.requests).toEqual([`${serverBaseUrl}/api/v1/settings/agent`])
  })

  it('tests that pages keep recording by default and when the server cannot answer', async () => {
    const on = createSessionReplaysSetting({
      resolveServerBaseUrl: async () => serverBaseUrl,
      fetch: settingsServer(() => ({})).fetch,
    })
    expect(await on.enabled()).toBe(true)

    const unreachable = createSessionReplaysSetting({
      resolveServerBaseUrl: async () => serverBaseUrl,
      fetch: settingsServer(() => new Error('connection refused')).fetch,
    })
    expect(await unreachable.enabled()).toBe(true)
  })

  it('tests that a change is picked up once the short cache expires', async () => {
    let enabled = true
    let clock = 0
    const server = settingsServer(() => ({ sessionReplaysEnabled: enabled }))
    const setting = createSessionReplaysSetting({
      resolveServerBaseUrl: async () => serverBaseUrl,
      fetch: server.fetch,
      now: () => clock,
    })

    expect(await setting.enabled()).toBe(true)
    enabled = false
    clock = 1_000
    expect(await setting.enabled()).toBe(true)
    expect(server.requests).toHaveLength(1)

    clock = 31_000
    expect(await setting.enabled()).toBe(false)
    expect(server.requests).toHaveLength(2)
  })
})
