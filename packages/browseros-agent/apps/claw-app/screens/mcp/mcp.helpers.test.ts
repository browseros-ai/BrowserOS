import { describe, expect, it } from 'bun:test'
import type { ConnectionState } from '@/modules/api/connections.hooks'
import { groupAgents, placementOf } from './mcp.helpers'

function agent(harness: string, installed: boolean): ConnectionState {
  return { harness, installed, message: '' } as ConnectionState
}

const names = (rows: ConnectionState[]) => rows.map((row) => row.harness)

describe('groupAgents', () => {
  it('groups by the current state on first load', () => {
    const rows = [agent('Codex', true), agent('Zed', false)]
    const { connected, notConnected } = groupAgents(rows, null)
    expect(names(connected)).toEqual(['Codex'])
    expect(names(notConnected)).toEqual(['Zed'])
  })

  it('keeps every agent in its first group after connecting or disconnecting', () => {
    const placement = placementOf([agent('Codex', true), agent('Zed', false)])
    const { connected, notConnected } = groupAgents(
      [agent('Codex', false), agent('Zed', true)],
      placement,
    )
    expect(names(connected)).toEqual(['Codex'])
    expect(connected[0]?.installed).toBe(false)
    expect(names(notConnected)).toEqual(['Zed'])
    expect(notConnected[0]?.installed).toBe(true)
  })

  it('places an agent it had not seen by its current state', () => {
    const placement = placementOf([agent('Codex', true)])
    const { connected } = groupAgents(
      [agent('Codex', true), agent('Cursor', true)],
      placement,
    )
    expect(names(connected)).toEqual(['Codex', 'Cursor'])
  })
})
