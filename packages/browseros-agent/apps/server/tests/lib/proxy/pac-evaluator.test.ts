import { afterEach, describe, expect, it } from 'bun:test'
import {
  evaluatePacText,
  evaluatePacUrl,
  parsePacResult,
  resetPacCache,
} from '../../../src/lib/proxy/pac-evaluator'

afterEach(() => {
  resetPacCache()
})

describe('parsePacResult', () => {
  it('maps PROXY to an http URL', () => {
    expect(parsePacResult('PROXY proxy.corp:8080')).toEqual({
      type: 'proxy',
      url: 'http://proxy.corp:8080',
    })
  })

  it('takes the first directive only', () => {
    expect(parsePacResult('PROXY proxy.corp:8080; DIRECT')).toEqual({
      type: 'proxy',
      url: 'http://proxy.corp:8080',
    })
  })

  it('maps HTTPS to an https URL', () => {
    expect(parsePacResult('HTTPS proxy.corp:8443')).toEqual({
      type: 'proxy',
      url: 'https://proxy.corp:8443',
    })
  })

  it('maps DIRECT', () => {
    expect(parsePacResult('DIRECT')).toEqual({ type: 'direct' })
  })

  it('marks SOCKS as unsupported', () => {
    expect(parsePacResult('SOCKS socks.corp:1080')).toEqual({
      type: 'unsupported',
    })
  })
})

describe('evaluatePacText', () => {
  it('returns the proxy verdict from the resolver', async () => {
    const verdict = await evaluatePacText('pac', 'https://api.openai.com/v1', {
      loadResolver: async () => async () => 'PROXY proxy.corp:8080',
    })
    expect(verdict).toEqual({
      kind: 'proxy',
      url: 'http://proxy.corp:8080',
    })
  })

  it('returns a direct verdict for DIRECT', async () => {
    const verdict = await evaluatePacText('pac', 'http://localhost:11434/', {
      loadResolver: async () => async () => 'DIRECT',
    })
    expect(verdict).toEqual({ kind: 'direct' })
  })

  it('returns failed when the runtime is unavailable', async () => {
    const verdict = await evaluatePacText('pac', 'https://example.com/', {
      loadResolver: async () => {
        throw new Error('no wasm')
      },
    })
    expect(verdict).toEqual({ kind: 'failed' })
  })

  it('returns failed when evaluation throws', async () => {
    const verdict = await evaluatePacText('pac', 'https://example.com/', {
      loadResolver: async () => async () => {
        throw new Error('bad pac')
      },
    })
    expect(verdict).toEqual({ kind: 'failed' })
  })

  it('returns failed for SOCKS results Bun cannot use', async () => {
    const verdict = await evaluatePacText('pac', 'https://example.com/', {
      loadResolver: async () => async () => 'SOCKS socks.corp:1080',
    })
    expect(verdict).toEqual({ kind: 'failed' })
  })
})

describe('evaluatePacUrl', () => {
  it('fetches the PAC once and caches it', async () => {
    let fetches = 0
    const deps = {
      fetchText: async () => {
        fetches += 1
        return 'pac-text'
      },
      loadResolver: async () => async () => 'PROXY proxy.corp:8080',
    }
    await evaluatePacUrl('http://wpad.corp/wpad.dat', 'https://a.com/', deps)
    await evaluatePacUrl('http://wpad.corp/wpad.dat', 'https://b.com/', deps)
    expect(fetches).toBe(1)
  })

  it('returns failed when the PAC fetch fails', async () => {
    const verdict = await evaluatePacUrl(
      'http://wpad.corp/wpad.dat',
      'https://a.com/',
      {
        fetchText: async () => undefined,
        loadResolver: async () => async () => 'PROXY proxy.corp:8080',
      },
    )
    expect(verdict).toEqual({ kind: 'failed' })
  })
})
