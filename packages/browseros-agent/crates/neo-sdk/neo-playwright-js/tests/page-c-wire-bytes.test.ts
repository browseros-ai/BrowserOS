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
      // Neo's own read of a page first seen mid-run (the new page's tree) is not in this
      // recording, which predates it; none of its scenarios reads frames itself.
      if (call.name === 'page.frames') return null
      const exchange = scenario.exchanges[next++]
      if (!exchange) throw new Error('the prelude sent an extra engine call')
      return JSON.parse(exchange.reply)
    })
    expect(next).toBe(scenario.exchanges.length)
    expect({
      ...actual,
      exchanges: actual.exchanges.filter(
        (exchange) => exchange.op !== 'page.frames',
      ),
    }).toEqual({
      exchanges: scenario.exchanges,
      result: scenario.result,
    })
  })
}

it("changes only storage command packaging, the void viewport receipt and page B's init scripts and disposables", () => {
  const expected = structuredClone(before)
  for (const scenario of expected.scenarios) {
    for (const exchange of scenario.exchanges) {
      // Page B (task 006): an init script's source or path travels at the top level as
      // upstream's `source`, a registration answers its bare id, and a dispose answers null.
      if (exchange.op === 'context.addInitScript') {
        const { script, ...args } = JSON.parse(exchange.args)
        exchange.args = JSON.stringify({ ...args, ...script })
      }
      if (
        exchange.op === 'context.addInitScript' ||
        exchange.op === 'context.exposeBinding'
      )
        exchange.reply = exchange.reply.replace(
          /"ok":\{"id":(\d+)\}/,
          '"ok":$1',
        )
      if (exchange.op === 'disposable.dispose')
        exchange.reply = exchange.reply.replace('"ok":{}', '"ok":null')
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
