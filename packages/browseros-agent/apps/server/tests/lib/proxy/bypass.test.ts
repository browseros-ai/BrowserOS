import { describe, expect, it } from 'bun:test'
import { DEFAULT_BYPASS, isBypassed } from '../../../src/lib/proxy/bypass'

describe('isBypassed', () => {
  it('matches exact hosts case-insensitively', () => {
    expect(isBypassed('Example.COM', '443', ['example.com'])).toBe(true)
    expect(isBypassed('other.com', '443', ['example.com'])).toBe(false)
  })

  it('matches subdomains for suffix patterns', () => {
    expect(isBypassed('app.corp', '443', ['.corp'])).toBe(true)
    expect(isBypassed('app.corp', '443', ['*.corp'])).toBe(true)
    expect(isBypassed('app.corp', '443', ['corp'])).toBe(true)
    expect(isBypassed('corp', '443', ['corp'])).toBe(true)
    expect(isBypassed('notcorp', '443', ['corp'])).toBe(false)
  })

  it('supports a global wildcard', () => {
    expect(isBypassed('anything.example', '80', ['*'])).toBe(true)
  })

  it('treats <local> as plain hostnames without a dot', () => {
    expect(isBypassed('ollama', '11434', ['<local>'])).toBe(true)
    expect(isBypassed('ollama.local', '11434', ['<local>'])).toBe(false)
  })

  it('honors host:port pairs', () => {
    expect(isBypassed('proxy.corp', '8080', ['proxy.corp:8080'])).toBe(true)
    expect(isBypassed('proxy.corp', '9090', ['proxy.corp:8080'])).toBe(false)
    expect(isBypassed('proxy.corp', '9090', ['proxy.corp'])).toBe(true)
  })

  it('matches IPv6 loopback with or without brackets', () => {
    expect(isBypassed('::1', '80', ['::1'])).toBe(true)
    expect(isBypassed('[::1]', '80', ['::1'])).toBe(true)
  })

  it('skips blank patterns', () => {
    expect(isBypassed('example.com', '443', ['', '  '])).toBe(false)
  })

  it('covers local LLM servers by default', () => {
    expect(DEFAULT_BYPASS).toContain('localhost')
    expect(isBypassed('localhost', '11434', DEFAULT_BYPASS)).toBe(true)
    expect(isBypassed('127.0.0.1', '11434', DEFAULT_BYPASS)).toBe(true)
  })
})
