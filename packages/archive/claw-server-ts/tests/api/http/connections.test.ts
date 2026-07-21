import { describe, expect, mock, test } from 'bun:test'
import { Harness } from '@browseros/claw-api'
import { createConnectionHandlers } from '../../../src/api/http/handlers/connections'
import { connection, httpTestApp } from './fixtures'

describe('connection HTTP handlers', () => {
  test('lists, connects, and disconnects harnesses', async () => {
    const connectHarness = mock(async () => connection)
    const disconnectHarness = mock(async () => ({
      ...connection,
      installed: false,
    }))
    const app = httpTestApp({
      connections: createConnectionHandlers({
        listConnections: async () => ({ items: [connection] }),
        connectHarness,
        disconnectHarness,
      }),
    })
    const list = await app.request('/api/v1/connections')
    expect(await list.json()).toEqual({ items: [connection] })

    const connect = await app.request('/api/v1/connections/Codex', {
      method: 'PUT',
    })
    expect(await connect.json()).toEqual(connection)
    expect(connectHarness).toHaveBeenCalledWith(Harness.Codex)

    const disconnect = await app.request('/api/v1/connections/Codex', {
      method: 'DELETE',
    })
    expect(await disconnect.json()).toEqual({
      ...connection,
      installed: false,
    })
    expect(disconnectHarness).toHaveBeenCalledWith(Harness.Codex)
  })

  test('rejects unknown and malformed encoded harnesses', async () => {
    const app = httpTestApp()
    for (const harness of ['Unknown', '%E0%A4%A']) {
      const response = await app.request(`/api/v1/connections/${harness}`, {
        method: 'PUT',
      })
      expect(response.status).toBe(404)
      expect(await response.json()).toMatchObject({ code: 'harness_not_found' })
    }
  })
})
