/**
 * The block a covered click's error carries: the cover's tree in the default form, cut at 1,500
 * units.
 */
import { cover, type Gap, tree } from './model'

const smile = '\u{1F600}'

const cookies = (text: string) =>
  tree([
    {
      role: 'dialog',
      name: 'Cookies',
      ref: 'e9',
      children: [
        { role: 'paragraph', children: [text] },
        { role: 'button', name: 'Accept', ref: 'e10' },
      ],
    },
  ])

export const coveredBlock: Gap = {
  snapshots: {
    // The dialog's line (28 units), the text's (10 and 1,430) and the button's (29), each with a newline, take exactly 1,500 units: the block is whole.
    'covered-block/at-limit': cookies('a'.repeat(1430)),
    // One unit past: the last line loses its last unit and the cut line follows.
    'covered-block/past-limit': cookies('a'.repeat(1431)),
    // The text's line ends at unit 1,500: the next line is not started, only the cut line follows.
    'covered-block/line-end': cookies('a'.repeat(1460)),
    // The text's line ends at unit 1,499: no room is left, only the cut line follows.
    'covered-block/room-zero': cookies('a'.repeat(1459)),
    // The text's line ends at unit 1,498: one unit of the next line is kept, its leading space.
    'covered-block/room-one': cookies('a'.repeat(1458)),
    // The cut falls between the two halves of a character: the half kept is written U+FFFD.
    'covered-block/inside-character': cookies(
      `${'a'.repeat(1459)}${smile}${'b'.repeat(10)}`,
    ),
    // The cut falls right after a whole character: it is kept whole.
    'covered-block/after-character': cookies(
      `${'a'.repeat(1458)}${smile}${'b'.repeat(10)}`,
    ),
    // A tree with no node: nothing renders, so the message carries no block.
    'covered-block/no-node': tree([]),
    // A ref holding raw line breaks prints over three lines, the middle one blank to
    // JavaScript's trim (a space, U+3000 and U+FEFF, which Rust's own trim keeps): it is not printed.
    'covered-block/blank-line': tree([
      {
        role: 'dialog',
        name: 'Cookies',
        ref: 'e9',
        children: [
          { role: 'button', name: 'Accept', ref: 'e10\n \u3000\uFEFF\ne11' },
        ],
      },
    ]),
    // A tree whose default form prints nothing (a separator alone): no block.
    'covered-block/prints-nothing': tree([
      { role: 'generic', children: [{ role: 'paragraph', children: ['|'] }] },
    ]),
  },
  cases: [
    ...[
      'at-limit',
      'past-limit',
      'line-end',
      'room-zero',
      'room-one',
      'inside-character',
      'after-character',
      'blank-line',
      'no-node',
      'prints-nothing',
    ].map((shape) => ({
      name: `covered-block/${shape}/cover`,
      snapshot: `covered-block/${shape}`,
      call: cover(),
    })),
    // The cover's tree is null: no block.
    { name: 'covered-block/null/cover', snapshot: null, call: cover() },
  ],
}
