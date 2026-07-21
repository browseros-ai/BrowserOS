import { describe, expect, mock, test } from 'bun:test'
import { createSessionHandlers } from '../../../src/api/http/handlers/sessions'
import { httpTestApp, sessionDetail, sessions } from './fixtures'

function handlers(
  overrides: Partial<Parameters<typeof createSessionHandlers>[0]> = {},
) {
  return createSessionHandlers({
    listSessions: () => sessions,
    getSession: () => sessionDetail,
    getSessionState: () => 'live',
    cancelSession: () => 0,
    ...overrides,
  })
}

describe('session HTTP handlers', () => {
  test('passes the complete validated list query to the session operation', async () => {
    const listSessions = mock(() => sessions)
    const app = httpTestApp({ sessions: handlers({ listSessions }) })
    const response = await app.request(
      '/api/v1/sessions?profileId=p&slug=codex&status=live&site=browseros.com&search=docs&since=0&cursor=2&limit=25',
    )
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual(sessions)
    expect(listSessions).toHaveBeenCalledWith({
      profileId: 'p',
      slug: 'codex',
      status: 'live',
      site: 'browseros.com',
      search: 'docs',
      since: 0,
      cursor: 2,
      limit: 25,
    })
  })

  test('rejects invalid status, since, cursor, and limit values', async () => {
    const app = httpTestApp({ sessions: handlers() })
    for (const query of [
      'status=active',
      'since=-1',
      'cursor=0',
      'limit=0',
      'limit=101',
    ]) {
      const response = await app.request(`/api/v1/sessions?${query}`)
      expect(response.status, query).toBe(400)
      expect(await response.json()).toMatchObject({ code: 'invalid_request' })
    }
  })

  test('distinguishes present and missing sessions', async () => {
    const present = await httpTestApp({ sessions: handlers() }).request(
      '/api/v1/sessions/session-live',
    )
    expect(present.status).toBe(200)
    expect(await present.json()).toEqual(sessionDetail)

    const missing = await httpTestApp({
      sessions: handlers({ getSession: () => null }),
    }).request('/api/v1/sessions/missing')
    expect(missing.status).toBe(404)
    expect(await missing.json()).toMatchObject({ code: 'session_not_found' })
  })

  test('distinguishes missing, ended, and idle live cancellation', async () => {
    for (const [state, status, code] of [
      ['missing', 404, 'session_not_found'],
      ['ended', 409, 'session_not_live'],
    ] as const) {
      const app = httpTestApp({
        sessions: handlers({ getSessionState: () => state }),
      })
      const response = await app.request('/api/v1/sessions/session-1/cancel', {
        method: 'POST',
      })
      expect(response.status).toBe(status)
      expect(await response.json()).toMatchObject({ code })
    }

    const cancelSession = mock(() => 0)
    const response = await httpTestApp({
      sessions: handlers({ cancelSession }),
    }).request('/api/v1/sessions/session-live/cancel', { method: 'POST' })
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual({ cancelled: 0 })
  })
})
