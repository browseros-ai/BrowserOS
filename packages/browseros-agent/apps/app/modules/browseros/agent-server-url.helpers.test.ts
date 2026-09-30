import { describe, expect, it } from 'bun:test'
import { resolveAgentServerUrl } from './agent-server-url.helpers'

/**
 * The retry exists for one window: the browser publishes the port pref shortly
 * after the extension loads, and a surface that asks first throws.
 */
describe('resolveAgentServerUrl', () => {
  it('returns the url without waiting when it is already published', async () => {
    const slept: number[] = []

    const url = await resolveAgentServerUrl({
      read: async () => 'http://127.0.0.1:9100',
      sleep: async (ms) => {
        slept.push(ms)
      },
    })

    expect(url).toBe('http://127.0.0.1:9100')
    expect(slept).toEqual([])
  })

  it('waits and asks again when the port is not published yet', async () => {
    let attempts = 0
    const slept: number[] = []

    const url = await resolveAgentServerUrl({
      read: async () => {
        attempts += 1
        if (attempts < 3) throw new Error('Server port not configured.')
        return 'http://127.0.0.1:9100'
      },
      retryDelayMs: 500,
      sleep: async (ms) => {
        slept.push(ms)
      },
    })

    expect(url).toBe('http://127.0.0.1:9100')
    expect(attempts).toBe(3)
    expect(slept).toEqual([500, 500])
  })

  it('surfaces the last failure once the attempts run out', async () => {
    let attempts = 0

    const resolving = resolveAgentServerUrl({
      read: async () => {
        attempts += 1
        throw new Error(`no port on attempt ${attempts}`)
      },
      maxAttempts: 3,
      sleep: async () => {},
    })

    await expect(resolving).rejects.toThrow('no port on attempt 3')
    expect(attempts).toBe(3)
  })

  it('does not sleep after the final attempt', async () => {
    const slept: number[] = []

    await expect(
      resolveAgentServerUrl({
        read: async () => {
          throw new Error('no port')
        },
        maxAttempts: 2,
        retryDelayMs: 500,
        sleep: async (ms) => {
          slept.push(ms)
        },
      }),
    ).rejects.toThrow('no port')

    expect(slept).toEqual([500])
  })

  it('wraps a non-Error rejection so callers always get an Error', async () => {
    await expect(
      resolveAgentServerUrl({
        read: async () => {
          throw 'port pref missing'
        },
        maxAttempts: 1,
        sleep: async () => {},
      }),
    ).rejects.toThrow('port pref missing')
  })
})
