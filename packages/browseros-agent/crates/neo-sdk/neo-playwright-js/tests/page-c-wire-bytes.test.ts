/** Replay the aligned page-C boundary against its pre-alignment recording, including exact JSON field order. */
import { expect, it } from 'bun:test'
import fixture from '../../neo-playwright/tests/data/page-c-aligned-wire.json'
import before from '../../neo-playwright/tests/data/page-c-wire.json'
import { recordPointerWire } from './support/pointer-wire-runtime'

for (const scenario of fixture.scenarios) {
  it(`pins aligned page-C request and reply bytes: ${scenario.name}`, async () => {
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

it('changes only storage command packaging and the void viewport receipt', () => {
  const expected = structuredClone(before)
  for (const scenario of expected.scenarios) {
    for (const exchange of scenario.exchanges) {
      if (
        exchange.op === 'webStorage.read' ||
        exchange.op === 'webStorage.write'
      ) {
        const args = JSON.parse(exchange.args)
        exchange.op = `webStorage.${args.action}`
        delete args.action
        exchange.args = JSON.stringify(args)
      }
      if (exchange.op === 'page.setViewportSize')
        exchange.reply = exchange.reply.replace('"ok":{}', '"ok":null')
    }
  }
  expect(fixture).toEqual(expected)
})
