import { expect, it } from 'bun:test'
import { renderSnapshotV1 } from '../src/prelude/snapshot-v1'
import type { SnapshotResult } from '../src/prelude/wire'
import { fields } from './support/field-values'

type Node = SnapshotResult['json'][number]
const render = (json: Node[], text: boolean) =>
  renderSnapshotV1(
    { json, partial: [] },
    { text, urls: false, maxTokens: 8000 },
  ).text

it('prints captured dropdown values and editor content with their field refs in both forms', () => {
  const expected = [
    '- combobox "Data Size" [ref=e101]: 1,000 Rows, 22 Cols',
    '- combobox "Theme" [ref=e106]: Quartz',
    '- combobox "Reference category" [ref=e278]: Frequently Used Tokens',
    '- textbox "Code Editor for App.js" [ref=e237]: "function MyButton() { return ( <button> I\'m a button </button> ); } export default function MyApp() { return ( <div> <h1>Welcome to my app</h1> <MyButton /> </div> ); }"',
  ].join('\n')
  for (const text of [true, false]) expect(render(fields, text)).toBe(expected)
})

it('does not repeat a field name as its value, with or without a ref', () => {
  for (const ref of [undefined, 'e1']) {
    const node: Node = {
      role: 'textbox',
      name: 'Note',
      ref,
      children: [{ role: 'paragraph', children: ['Note'] }],
    }
    for (const text of [true, false])
      expect(render([node], text)).toBe(
        `- textbox "Note"${ref ? ` [ref=${ref}]` : ''}`,
      )
  }
})

it('keeps native select options as the choices without duplicating them as a value', () => {
  const node: Node = {
    role: 'combobox',
    name: 'Quantity',
    ref: 'e2',
    children: [
      { role: 'option', name: '1', selected: true },
      { role: 'option', name: '2' },
    ],
  }
  for (const text of [true, false])
    expect(render([node], text)).toBe(
      '- combobox "Quantity" [ref=e2]:\n  - option "1" [selected]\n  - option "2"',
    )
})
