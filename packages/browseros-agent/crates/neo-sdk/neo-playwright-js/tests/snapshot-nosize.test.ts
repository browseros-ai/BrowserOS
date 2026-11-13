import { expect, it } from 'bun:test'
import { renderAriaSnapshotAsYaml } from '../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer'
import { renderSnapshotV1 } from '../src/prelude/snapshot-v1'
import type { SnapshotResult } from '../src/prelude/wire'
import { loadPrelude, SEED } from './support/prelude-runtime'
import { recordWireBytes } from './support/wire-byte-runtime'

type Node = SnapshotResult['json'][number]
const render = (json: Node[], text: boolean) =>
  renderSnapshotV1(
    { json, partial: [] },
    { text, urls: false, maxTokens: 8000 },
  ).text

it('keeps every ref-less field in both forms with Playwright states and values', () => {
  const roles: Node['role'][] = [
    'textbox',
    'searchbox',
    'spinbutton',
    'slider',
    'combobox',
    'listbox',
    'checkbox',
    'radio',
    'switch',
    'menuitemcheckbox',
    'menuitemradio',
  ]
  for (const role of roles) {
    const node: Node = {
      role,
      name: 'Code',
      disabled: true,
      active: true,
      text: '604918',
    }
    const expected = renderAriaSnapshotAsYaml([node])
    for (const text of [true, false])
      expect(render([node], text)).toBe(expected)
  }
  const states: Node['checked'][] = [true, undefined, 'mixed']
  for (const checked of states) {
    const node: Node = { role: 'checkbox', name: 'Email me updates', checked }
    expect(render([node], true)).toBe(renderAriaSnapshotAsYaml([node]))
    expect(render([node], false)).toBe(renderAriaSnapshotAsYaml([node]))
  }
})

it('prints the row, digit, label, checkbox and radio shapes without repeating field content', () => {
  const drawing = (children: Node[]): Node => ({
    role: 'generic',
    ref: 'e4',
    cursor: 'pointer',
    children,
  })
  const fixtures: [Node[], string][] = [
    [
      [
        { role: 'text', text: 'Enter the code we sent you' },
        {
          role: 'textbox',
          name: 'Verification code',
          active: true,
          text: '604918',
        },
        drawing([]),
      ],
      '- text: Enter the code we sent you\n- textbox "Verification code" [active]: "604918"\n- generic [ref=e4] [cursor=pointer]',
    ],
    [
      [drawing([{ role: 'textbox', name: 'Digit 1' }])],
      '- generic [ref=e4] [cursor=pointer]\n- textbox "Digit 1"',
    ],
    [
      [
        { role: 'textbox', name: 'Code' },
        drawing([{ role: 'text', text: 'Code' }]),
      ],
      '- textbox "Code"\n- generic [ref=e4] [cursor=pointer]: Code',
    ],
    [
      [
        drawing([
          { role: 'text', text: 'Email me updates' },
          { role: 'checkbox', name: 'Email me updates', checked: true },
        ]),
      ],
      '- generic [ref=e4] [cursor=pointer]: Email me updates\n- checkbox "Email me updates" [checked]',
    ],
    [
      [
        drawing([
          { role: 'text', text: 'Yearly' },
          { role: 'radio', name: 'Yearly', checked: true },
        ]),
      ],
      '- generic [ref=e4] [cursor=pointer]: Yearly\n- radio "Yearly" [checked]',
    ],
    [
      [{ role: 'spinbutton', name: 'Verification code' }],
      '- spinbutton "Verification code"',
    ],
  ]
  for (const [tree, expected] of fixtures) {
    expect(render(tree, true)).toBe(expected)
    expect(render(tree, false)).toBe(
      expected
        .split('\n')
        .filter((line) => !line.startsWith('- text:'))
        .join('\n'),
    )
  }
})

it('keeps native choices but preserves non-field controls without refs as page text', () => {
  const choices: Node = {
    role: 'combobox',
    name: 'Quantity',
    children: [
      { role: 'option', name: '1', selected: true },
      { role: 'option', name: '2' },
    ],
  }
  for (const text of [true, false])
    expect(render([choices], text)).toBe(
      '- combobox "Quantity":\n  - option "1" [selected]\n  - option "2"',
    )
  const others: Node[] = [
    { role: 'button', name: 'Unavailable' },
    { role: 'option', name: 'Choice' },
  ]
  expect(render(others, true)).toBe('- text: Unavailable Choice')
  expect(render(others, false)).toBe('')
})

it('sends ref as a strict attached read and returns the string unmodified', async () => {
  const calls: { name: string; input: unknown }[] = []
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => {
      if (call.name !== 'locator.ref') return null
      calls.push(call)
      return 'f2n3'
    },
  })
  expect(
    await runtime.run(
      "return await page.frameLocator('iframe').getByRole('textbox', { name: 'Code', exact: true }).ref({ timeout: 1234 })",
    ),
  ).toBe('f2n3')
  expect(calls).toHaveLength(1)
  expect(calls[0].input).toMatchObject({
    call: { api: 'locator.ref', timeoutMs: 1234 },
    strict: 'strict',
    target: { $neo: 'locator' },
  })
})

it('pins locator.ref wire bytes without introducing a snapshot or an action', async () => {
  const { exchanges, result } = await recordWireBytes(
    "return await page.getByRef('e5').ref({ timeout: 1234 })",
    (call) => (call.name === 'locator.ref' ? 'e5' : null),
  )
  expect(exchanges).toHaveLength(1)
  expect(exchanges[0].op).toBe('locator.ref')
  expect(exchanges[0].args).toBe(
    '{"call":{"api":"locator.ref","timeoutMs":1234},"target":{"$neo":"locator","page":{"$neo":"page","tab":1007},"frame":null,"chunks":[{"parts":[{"name":"aria-ref","body":"e5","source":"e5"}]}],"display":"locator(\'aria-ref=e5\')"},"strict":"strict"}',
  )
  expect(result).toBe('"e5"')
})

it('keeps ref available under the conformance switch', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    upstreamSnapshots: true,
    answer: (call) => (call.name === 'locator.ref' ? 'n1' : null),
  })
  expect(await runtime.run("return await page.locator('input').ref()")).toBe(
    'n1',
  )
  expect(runtime.calls.map((call) => call.name)).toEqual(['locator.ref'])
})
