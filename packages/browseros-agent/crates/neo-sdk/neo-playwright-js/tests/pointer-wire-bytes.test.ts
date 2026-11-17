/** The engine-facing pointer contract captured before typed route migration, byte for byte. */
import { expect, it } from 'bun:test'
import fixture from '../../neo-playwright/tests/data/pointer-wire.json'
import { recordWireBytes } from './support/wire-byte-runtime'

for (const scenario of fixture.scenarios) {
  it(`pins aligned engine op, argument and recorded reply bytes: ${scenario.name}`, async () => {
    let next = 0
    const actual = await recordWireBytes(scenario.script, () => {
      const exchange = scenario.exchanges[next++]
      if (!exchange) throw new Error('the prelude sent an extra engine call')
      // Alignment changes only the approved request fields. Replaying the original reply,
      // including its observation envelope, also pins the script's value and navigation cut.
      return JSON.parse(exchange.reply)
    })
    expect(next).toBe(scenario.exchanges.length)
    expect(actual).toEqual({
      exchanges: scenario.exchanges.map(({ op, args, reply }) => ({
        op,
        args,
        reply,
      })),
      result: scenario.result,
    })
  })
}

it('byte-pins every migrated pointer op and the smooth-scroll retry operations', () => {
  const ops = new Set(
    fixture.scenarios.flatMap((scenario) =>
      scenario.exchanges.map((exchange) => exchange.op),
    ),
  )
  for (const op of [
    'locator.click',
    'locator.dblclick',
    'locator.hover',
    'locator.check',
    'locator.uncheck',
    'locator.dragTo',
    'locator.drop',
  ])
    expect(ops.has(op)).toBe(true)
  const scroll = fixture.scenarios.find(
    (scenario) => scenario.name === 'smooth-scroll retry operations',
  )
  expect(scroll).toBeDefined()
  const scrollOps = new Set(scroll?.exchanges.map((exchange) => exchange.op))
  for (const op of ['page.setContent', 'locator.click', 'page.evaluate'])
    expect(scrollOps.has(op)).toBe(true)
})
