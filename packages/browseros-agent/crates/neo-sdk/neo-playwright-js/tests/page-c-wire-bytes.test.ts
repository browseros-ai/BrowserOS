/** Replay the page-C boundary captured on the base, including exact JSON field order. */
import { expect, it } from 'bun:test'
import fixture from '../../neo-playwright/tests/data/page-c-wire.json'
import { recordPointerWire } from './support/pointer-wire-runtime'

for (const scenario of fixture.scenarios) {
  it(`preserves page-C request and reply bytes: ${scenario.name}`, async () => {
    let next = 0
    const actual = await recordPointerWire(scenario.script, (call) => {
      // The idle binding pump has no reply in the recording; disposal runs beside it.
      if (call.name === 'events.next') return new Promise(() => {})
      const exchange = scenario.exchanges[next++]
      if (!exchange) throw new Error('the prelude sent an extra engine call')
      return JSON.parse(exchange.reply)
    })
    expect(next).toBe(scenario.exchanges.length)
    expect(actual).toEqual({
      exchanges: scenario.exchanges,
      result: scenario.result,
    })
  })
}
