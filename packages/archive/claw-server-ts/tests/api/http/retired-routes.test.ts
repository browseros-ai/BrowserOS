import { describe, expect, test } from 'bun:test'
import { createServer } from '../../../src/server'

const retiredRoutes = [
  ['GET', '/system/version'],
  ['GET', '/system/url'],
  ['GET', '/system/telemetry'],
  ['POST', '/system/telemetry'],
  ['POST', '/agents/:agentId/cancel'],
  ['GET', '/tabs/activity'],
  ['GET', '/connections'],
  ['POST', '/connections/:harness/connect'],
  ['POST', '/connections/:harness/disconnect'],
  ['GET', '/audit/dispatches'],
  ['GET', '/audit/tasks'],
  ['GET', '/audit/tasks/:sessionId'],
  ['GET', '/audit/screenshot/:dispatchId'],
  ['GET', '/recordings/health'],
  ['POST', '/recordings/tabs/:tabId/events'],
  ['GET', '/audit/replays/:sessionId'],
  ['GET', '/audit/replays/:sessionId/meta'],
  ['GET', '/api/v1/tabs'],
  ['GET', '/api/v1/tabs/:pageId/preview'],
  ['GET', '/api/v1/sessions/:sessionId/browser-tabs/:browserTabId/preview'],
  ['GET', '/api/v1/dispatches/:dispatchId/screenshot'],
] as const

describe('retired REST routes', () => {
  const app = createServer()

  for (const [method, path] of retiredRoutes) {
    test(`${method} ${path}`, () => {
      expect(
        app.routes.some(
          (route) => route.method === method && route.path === path,
        ),
      ).toBe(false)
    })
  }

  test('retired canonical tab URLs return a bare router 404', async () => {
    for (const path of [
      '/api/v1/tabs',
      '/api/v1/tabs/7/preview',
      '/api/v1/sessions/session-1/browser-tabs/7/preview',
      '/api/v1/dispatches/1/screenshot',
    ]) {
      const response = await app.request(`http://localhost${path}`)
      expect(response.status).toBe(404)
      expect(response.headers.get('content-type') ?? '').not.toContain(
        'application/json',
      )
    }
  })
})
