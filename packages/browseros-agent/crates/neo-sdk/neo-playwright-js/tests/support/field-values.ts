/** Captured field subtrees from ag-grid, regex101 and React docs, before any rendering. */
import type { SnapshotResult } from '../../src/prelude/wire'

export const fields: SnapshotResult['json'] = [
  {
    role: 'combobox',
    name: 'Data Size',
    ref: 'e101',
    cursor: 'pointer',
    children: [
      {
        role: 'generic',
        text: '1,000 Rows, 22 Cols',
      },
    ],
  },
  {
    role: 'combobox',
    name: 'Theme',
    ref: 'e106',
    cursor: 'pointer',
    children: [
      {
        role: 'generic',
        text: 'Quartz',
      },
    ],
  },
  {
    role: 'combobox',
    name: 'Reference category',
    ref: 'e278',
    cursor: 'pointer',
    children: [
      {
        role: 'generic',
        ref: 'e279',
        text: 'Frequently Used Tokens',
      },
    ],
  },
  {
    role: 'textbox',
    name: 'Code Editor for App.js',
    ref: 'e237',
    children: [
      {
        role: 'generic',
        ref: 'e238',
        text: "function MyButton() { return ( <button> I'm a button </button> ); } export default function MyApp() { return ( <div> <h1>Welcome to my app</h1> <MyButton /> </div> ); }",
      },
    ],
  },
]
