import { afterEach, describe, expect, it, mock } from 'bun:test'

mock.module('@/modules/analytics/telemetry.hooks', () => ({
  useTelemetryState: () => ({ data: undefined, isError: false }),
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

const { priceForId, readJoined, rememberJoined } = await import(
  './ultrafast-price'
)

afterEach(() => {
  for (const key of Object.keys(memoryStore)) delete memoryStore[key]
})

describe('priceForId', () => {
  it('gives the same id the same price every time', () => {
    const id = '2f0c6a1e-8a51-4d6b-9a0f-3f6c1f1f2b7e'
    expect(priceForId(id)).toBe(priceForId(id))
  })

  it('splits ids roughly evenly between $9 and $19', () => {
    const counts = { 9: 0, 19: 0 }
    for (let i = 0; i < 2000; i++) {
      counts[priceForId(crypto.randomUUID())]++
    }
    expect(counts[9]).toBeGreaterThan(800)
    expect(counts[19]).toBeGreaterThan(800)
  })
})

describe('joined flag', () => {
  it('remembers a signup', () => {
    expect(readJoined()).toBe(false)
    rememberJoined()
    expect(readJoined()).toBe(true)
  })
})
