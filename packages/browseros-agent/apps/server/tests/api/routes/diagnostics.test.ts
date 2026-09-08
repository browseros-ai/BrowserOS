import { describe, expect, it } from 'bun:test'
import {
  createDiagnosticsRoute,
  parseOsRelease,
} from '../../../src/api/routes/diagnostics'

describe('system diagnostics metadata', () => {
  it('returns only public metadata and the running server version', async () => {
    const route = createDiagnosticsRoute('test-build')
    const response = await route.request('/')
    const body = await response.json()
    expect(response.status).toBe(200)
    expect(body.version).toBe('test-build')
    expect(Object.keys(body).sort()).toEqual(['os', 'osVersion', 'version'])
    expect(typeof body.os).toBe('string')
  })
  it('reads Linux distribution metadata without treating it as shell code', () => {
    expect(
      parseOsRelease('NAME="Ubuntu"\nVERSION_ID="24.04"\nOTHER=$(bad)'),
    ).toEqual({ os: 'Ubuntu', osVersion: '24.04' })
    expect(parseOsRelease('NAME=Fedora\nVERSION_ID=40')).toEqual({
      os: 'Fedora',
      osVersion: '40',
    })
    expect(parseOsRelease('')).toEqual({ os: 'Linux', osVersion: null })
  })
})
