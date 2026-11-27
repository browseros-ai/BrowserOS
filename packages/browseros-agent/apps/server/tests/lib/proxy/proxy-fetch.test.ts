import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { createProxiedFetch } from '../../../src/lib/proxy/proxy-fetch'

function stubInner(seen: Array<Record<string, unknown>>) {
  return (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url =
      typeof input === 'string' || input instanceof URL
        ? input.toString()
        : input.url
    seen.push({ input: url, ...(init as Record<string, unknown>) })
    return new Response('ok')
  }) as typeof fetch
}

// An empty env isolates tests from whatever proxy the runner has set.
const NO_ENV = { env: {} as NodeJS.ProcessEnv }

describe('createProxiedFetch', () => {
  it('injects the resolved proxy into init', async () => {
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      ...NO_ENV,
      resolve: async () => 'http://proxy.corp:8080',
    })
    const response = await proxied('https://api.openai.com/v1', {
      method: 'POST',
    })
    expect(await response.text()).toBe('ok')
    expect(seen[0]).toMatchObject({
      method: 'POST',
      proxy: 'http://proxy.corp:8080',
    })
  })

  it('passes through untouched when no proxy resolves', async () => {
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      ...NO_ENV,
      resolve: async () => undefined,
    })
    await proxied('http://127.0.0.1:11434/v1/models')
    expect(seen[0]).not.toHaveProperty('proxy')
  })

  it('fails open to direct when resolution throws', async () => {
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      ...NO_ENV,
      resolve: async () => {
        throw new Error('boom')
      },
    })
    await proxied('https://api.openai.com/v1')
    expect(seen[0]).not.toHaveProperty('proxy')
  })

  it('keeps an explicitly set proxy instead of overriding it', async () => {
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      ...NO_ENV,
      resolve: async () => 'http://other:8080',
    })
    await proxied('https://api.openai.com/v1', {
      proxy: 'http://explicit:8080',
    } as RequestInit)
    expect(seen[0]).toMatchObject({ proxy: 'http://explicit:8080' })
  })

  it('accepts Request objects as input', async () => {
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      ...NO_ENV,
      resolve: async () => 'http://proxy.corp:8080',
    })
    await proxied(new Request('https://api.openai.com/v1'))
    expect(seen[0]).toMatchObject({
      input: 'https://api.openai.com/v1',
      proxy: 'http://proxy.corp:8080',
    })
  })
})

describe('NO_PROXY seeding', () => {
  const saved = { ...process.env }

  beforeEach(() => {
    delete process.env.HTTP_PROXY
    delete process.env.HTTPS_PROXY
    delete process.env.NO_PROXY
    delete process.env.no_proxy
  })

  afterEach(() => {
    for (const key of ['HTTP_PROXY', 'HTTPS_PROXY', 'NO_PROXY', 'no_proxy']) {
      delete process.env[key]
    }
    Object.assign(process.env, saved)
  })

  it('seeds the bypassed hostname so Bun honors DIRECT', async () => {
    process.env.HTTP_PROXY = 'http://proxy.corp:8080'
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      resolve: async () => undefined,
    })
    await proxied('http://ollama:11434/v1/models')
    expect(seen[0]).not.toHaveProperty('proxy')
    expect(process.env.NO_PROXY).toBe('ollama')
  })

  it('does not seed when no env proxy is configured', async () => {
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      resolve: async () => undefined,
    })
    await proxied('http://ollama:11434/v1/models')
    expect(process.env.NO_PROXY).toBeUndefined()
  })

  it('does not seed when a proxy resolves', async () => {
    process.env.HTTP_PROXY = 'http://proxy.corp:8080'
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      resolve: async () => 'http://proxy.corp:8080',
    })
    await proxied('https://api.openai.com/v1')
    expect(process.env.NO_PROXY).toBeUndefined()
  })

  it('does not duplicate an already-listed hostname', async () => {
    process.env.HTTP_PROXY = 'http://proxy.corp:8080'
    process.env.NO_PROXY = 'ollama,example.com'
    const proxied = createProxiedFetch(stubInner([]), {
      resolve: async () => undefined,
    })
    await proxied('http://Ollama:11434/v1/models')
    expect(process.env.NO_PROXY).toBe('ollama,example.com')
  })

  it('never mutates the environment for an injected fake env', async () => {
    const seen: Array<Record<string, unknown>> = []
    const proxied = createProxiedFetch(stubInner(seen), {
      env: { HTTP_PROXY: 'http://proxy.corp:8080' } as NodeJS.ProcessEnv,
      resolve: async () => undefined,
    })
    await proxied('http://ollama:11434/v1/models')
    expect(process.env.NO_PROXY).toBeUndefined()
  })
})
