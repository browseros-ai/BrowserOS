import { afterEach, beforeAll, describe, expect, it, mock } from 'bun:test'

mock.module('@/modules/analytics/telemetry.hooks', () => ({
  useTelemetryState: () => ({ data: undefined, isPending: true }),
}))

const memoryStore: Record<string, string> = {}
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key: string) => memoryStore[key] ?? null,
    setItem: (key: string, value: string) => {
      memoryStore[key] = value
    },
    removeItem: (key: string) => {
      delete memoryStore[key]
    },
  },
})

let mod: typeof import('./ultrafast-price')

beforeAll(async () => {
  mod = await import('./ultrafast-price')
})

afterEach(() => {
  for (const key of Object.keys(memoryStore)) delete memoryStore[key]
})

describe('priceForId', () => {
  it('gives the same id the same price every time', () => {
    const id = '2f0c6a1e-8a51-4d6b-9a0f-3f6c1f1f2b7e'
    expect(mod.priceForId(id)).toBe(mod.priceForId(id))
  })

  it('splits ids roughly evenly between $10 and $20', () => {
    const counts = { 10: 0, 20: 0 }
    for (let i = 0; i < 2000; i++) {
      counts[mod.priceForId(crypto.randomUUID())]++
    }
    expect(counts[10]).toBeGreaterThan(800)
    expect(counts[20]).toBeGreaterThan(800)
  })
})

describe('fallbackPrice', () => {
  it('picks once and keeps that price', () => {
    expect(mod.fallbackPrice(() => 0.9)).toBe(20)
    expect(mod.fallbackPrice(() => 0.1)).toBe(20)
  })

  it('ignores a stored value outside the test prices', () => {
    memoryStore['ultrafastWaitlistPrice:v1'] = '15'
    expect(mod.fallbackPrice(() => 0.1)).toBe(10)
  })
})

describe('joined flag', () => {
  it('remembers a signup', () => {
    expect(mod.readJoined()).toBe(false)
    mod.rememberJoined()
    expect(mod.readJoined()).toBe(true)
  })
})
