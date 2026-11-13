/** Only Add's recorded uid envelope changes; inputs, other replies and script results stay. */
import { expect, it } from 'bun:test'
import fixture from '../../neo-playwright/tests/data/locator-handler-wire.json'
import { recordLocatorHandlerWire } from './support/locator-handler-wire-runtime'

for (const scenario of fixture.scenarios) {
  it(`only unwraps the add reply in locator-handler transport bytes: ${scenario.name}`, async () => {
    const exchanges = scenario.exchanges.map((exchange) => {
      if (exchange.op !== 'page.addLocatorHandler') return exchange
      const reply = JSON.parse(exchange.reply)
      return {
        ...exchange,
        reply: JSON.stringify({ ...reply, ok: reply.ok.uid }),
      }
    })
    expect(await recordLocatorHandlerWire(scenario.script)).toEqual({
      exchanges,
      result: scenario.result,
    })
  })
}
