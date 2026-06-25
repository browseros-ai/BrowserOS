import { describe, expect, it } from 'bun:test'
import {
  PROXY_PORT_MAX,
  PROXY_PORT_MIN,
  parseProxyPort,
} from './server-port-editor.helpers'

describe('parseProxyPort', () => {
  it('accepts a valid in-range port', () => {
    expect(parseProxyPort('9004')).toEqual({ ok: true, port: 9004 })
  })

  it('accepts the inclusive boundaries', () => {
    expect(parseProxyPort(String(PROXY_PORT_MIN))).toEqual({
      ok: true,
      port: 9000,
    })
    expect(parseProxyPort(String(PROXY_PORT_MAX))).toEqual({
      ok: true,
      port: 9999,
    })
  })

  it('rejects ports outside the range with a message naming the bounds', () => {
    const low = parseProxyPort('8999')
    const high = parseProxyPort('10000')
    expect(low.ok).toBe(false)
    expect(high.ok).toBe(false)
    if (!low.ok) {
      expect(low.error).toContain('9000')
      expect(low.error).toContain('9999')
    }
    if (!high.ok) {
      expect(high.error).toContain('9000')
      expect(high.error).toContain('9999')
    }
  })

  it('rejects empty or whitespace-only input', () => {
    expect(parseProxyPort('').ok).toBe(false)
    expect(parseProxyPort('   ').ok).toBe(false)
  })

  it('rejects non-numeric input', () => {
    expect(parseProxyPort('90a4').ok).toBe(false)
    expect(parseProxyPort('abc').ok).toBe(false)
  })

  it('rejects non-integer input instead of truncating', () => {
    expect(parseProxyPort('9000.5').ok).toBe(false)
  })

  it('trims surrounding whitespace', () => {
    expect(parseProxyPort('  9100 ')).toEqual({ ok: true, port: 9100 })
  })
})
