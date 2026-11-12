import { expect, it } from 'bun:test'
import { inputExamples } from '../scripts/input-examples'
import { recordEveryOp } from '../scripts/wire-examples'

it('derives bag keys and flat optional fields without treating selectOption data as a bag', async () => {
  const { optionalKeys, samples } = await inputExamples()
  expect(optionalKeys['locator.fill']).toContain('force')
  expect(optionalKeys['locator.selectOption']).toEqual(['force'])
  expect(optionalKeys['page.goto']).toContain('referer')
  expect(optionalKeys['page.ariaSnapshot']).toContain('boxes')
  for (const keys of Object.values(optionalKeys))
    for (const address of ['target', 'strict', 'page', 'frame', 'call'])
      expect(keys).not.toContain(address)
  expect(samples['locator.fill']?.force).toBeTypeOf('boolean')
  expect(samples['locator.dragTo']?.sourcePosition).toEqual({ x: 1, y: 1 })
})

it('records public lifecycle paths as well as ordinary option arguments', async () => {
  const calls = await recordEveryOp()
  expect(
    calls.some(
      (call) => call.name === 'events.listen' && call.input.retiredTab === 1007,
    ),
  ).toBe(true)
  expect(
    calls.some(
      (call) =>
        call.name === 'page.resolveLocatorHandler' &&
        call.input.remove === true,
    ),
  ).toBe(true)
  expect(
    calls.some(
      (call) =>
        call.name === 'context.exposeBinding' && call.input.noGlobal === true,
    ),
  ).toBe(true)
})
