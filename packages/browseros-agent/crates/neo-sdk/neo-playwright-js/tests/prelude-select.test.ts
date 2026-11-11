// selectOption's options as upstream's client sends them: a string is `valueOrLabel`, and an
// object keeps every attribute it names, so the page selects the option matching all of them
// (SO-008, "should select single option by multiple attributes").
import { expect, it } from 'bun:test'
import { loadPrelude, SEED } from './support/prelude-runtime'

const optionsSent = async (values: string) => {
  const runtime = loadPrelude({ seed: SEED, answer: () => [] })
  await runtime.run(`await page.selectOption('select', ${values})`)
  return runtime.sent('locator.selectOption').map((call) => call.input.options)
}

it('tests that an option keeps every attribute the script named', async () => {
  expect(await optionsSent(`{ value: 'green', label: 'Green' }`)).toEqual([
    [{ value: 'green', label: 'Green' }],
  ])
})

it('tests that strings, single attributes and an index each keep their own shape', async () => {
  expect(
    await optionsSent(
      `['blue', { label: 'Red' }, { index: 2 }, { valueOrLabel: 'x', index: 1 }]`,
    ),
  ).toEqual([
    [
      { valueOrLabel: 'blue' },
      { label: 'Red' },
      { index: 2 },
      { valueOrLabel: 'x', index: 1 },
    ],
  ])
})

it("tests that wrong types are refused with upstream's validator messages, before anything is sent", async () => {
  const runtime = loadPrelude({ seed: SEED, answer: () => [] })
  const messages = await runtime.run(`
    const tries = [12, true, { value: 12 }, { label: 12 }, { index: '12' }, { index: NaN }]
    const out = []
    for (const value of tries)
      out.push(await page.selectOption('select', value).then(() => 'sent', (e) => e.message))
    return out
  `)
  expect(messages).toEqual([
    'page.selectOption: options[0]: expected object, got number',
    'page.selectOption: options[0]: expected object, got boolean',
    'page.selectOption: options[0].value: expected string, got number',
    'page.selectOption: options[0].label: expected string, got number',
    'page.selectOption: options[0].index: expected integer, got string',
    'page.selectOption: options[0].index: expected integer, got float NaN',
  ])
  expect(
    runtime.calls.filter((call) => call.name === 'locator.selectOption'),
  ).toEqual([])
})
