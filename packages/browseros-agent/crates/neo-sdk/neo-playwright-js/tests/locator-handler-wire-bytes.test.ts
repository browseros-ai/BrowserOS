/** Before-migration recordings pin all four ops, including background replies and getters. */
import { expect, it } from 'bun:test'
import fixture from '../../neo-playwright/tests/data/locator-handler-wire.json'
import { recordLocatorHandlerWire } from './support/locator-handler-wire-runtime'

for (const scenario of fixture.scenarios) {
  it(`preserves locator-handler transport bytes: ${scenario.name}`, async () => {
    expect(await recordLocatorHandlerWire(scenario.script)).toEqual({
      exchanges: scenario.exchanges,
      result: scenario.result,
    })
  })
}
