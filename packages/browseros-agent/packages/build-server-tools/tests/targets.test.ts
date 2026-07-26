import { describe, expect, it } from 'bun:test'

import { resolveTargets } from '../src'

describe('target resolution', () => {
  it('resolves all production server targets', () => {
    expect(resolveTargets('all').map((target) => target.id)).toEqual([
      'linux-x64',
      'linux-arm64',
      'windows-x64',
      'darwin-arm64',
      'darwin-x64',
    ])
  })

  it('reports the full allowed list for invalid target ids', () => {
    expect(() => resolveTargets('darwin-arm64,plan9-x64')).toThrow(
      'Available: linux-x64, linux-arm64, windows-x64, darwin-arm64, darwin-x64, all',
    )
  })
})
