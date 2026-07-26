import { describe, expect, it } from 'bun:test'
import { formatOffset, hostOf } from './screenshot.helpers'

describe('formatOffset', () => {
  it('renders sub-second offsets in milliseconds', () => {
    expect(formatOffset(0)).toBe('0ms')
    expect(formatOffset(500)).toBe('500ms')
    expect(formatOffset(999)).toBe('999ms')
  })

  it('renders sub-minute offsets in seconds with one decimal', () => {
    expect(formatOffset(1000)).toBe('1.0s')
    expect(formatOffset(1500)).toBe('1.5s')
    expect(formatOffset(59900)).toBe('59.9s')
  })

  it('renders minute-plus offsets as m + zero-padded seconds', () => {
    expect(formatOffset(60000)).toBe('1m00s')
    expect(formatOffset(65000)).toBe('1m05s')
    expect(formatOffset(600000)).toBe('10m00s')
  })
})

describe('hostOf', () => {
  it('strips the scheme, path, and leading www.', () => {
    expect(hostOf('https://www.example.com/admin?q=1')).toBe('example.com')
    expect(hostOf('https://admin.google.com')).toBe('admin.google.com')
  })

  it('returns an empty string for null or unparseable input', () => {
    expect(hostOf(null)).toBe('')
    expect(hostOf('not a url')).toBe('')
  })
})
