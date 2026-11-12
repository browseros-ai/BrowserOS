/** The engine-facing form, key and file contract captured before typed route migration, byte for byte. */
import { expect, it } from 'bun:test'
import fixture from '../../neo-playwright/tests/data/form-wire.json'
import { recordWireBytes } from './support/wire-byte-runtime'

for (const scenario of fixture.scenarios) {
  it(`preserves engine op, argument and recorded reply bytes: ${scenario.name}`, async () => {
    let next = 0
    const actual = await recordWireBytes(scenario.script, () => {
      const exchange = scenario.exchanges[next++]
      if (!exchange) throw new Error('the prelude sent an extra engine call')
      // Replay the before-build reply, including its observation envelope. The bootstrap
      // consumes the same engine bytes; onWire observes the actual request and reply strings.
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

it('byte-pins all ten form, key, file and event operations', () => {
  const ops = new Set(
    fixture.scenarios.flatMap((s) => s.exchanges.map((e) => e.op)),
  )
  for (const op of [
    'fill',
    'press',
    'pressSequentially',
    'selectOption',
    'selectText',
    'focus',
    'blur',
    'setInputFiles',
    'dispatchEvent',
    'scrollIntoViewIfNeeded',
  ])
    expect(ops.has(`locator.${op}`)).toBe(true)
})

it('records only delay when a script passes pressSequentially noWaitAfter', () => {
  const scenario = fixture.scenarios.find(
    (s) => s.name === 'pressSequentially drops noWaitAfter',
  )
  const exchange = scenario?.exchanges.find(
    (e) => e.op === 'locator.pressSequentially',
  )
  expect(exchange).toBeDefined()
  expect(JSON.parse(exchange?.args ?? '{}').options).toEqual({ delay: 7 })
})
