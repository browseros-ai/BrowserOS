/**
 * Frames Rust could not read: `# partial:` under their iframe, or at the end with no iframe line.
 */
import { calls, element, full, type Gap, playwright, tree } from './model'

const PAY = 'https://pay.example/card'
const MAP = 'https://map.example/'

const threeFormats = () => ({
  playwright: playwright(),
  full: full(),
  element: element(),
})

export const partialFrames: Gap = {
  snapshots: {
    // Each comment one level under its iframe's line: an empty iframe's, and one with fallback text.
    'partial-frames/under-iframe': tree(
      [
        {
          role: 'main',
          ref: 'e1',
          children: [
            { role: 'iframe', ref: 'e2' },
            { role: 'iframe', ref: 'e3', text: 'Frames are off.' },
          ],
        },
      ],
      {
        partial: [
          { ref: 'e2', frame: PAY, reason: 'timeout after 2000 ms' },
          { ref: 'e3', frame: MAP, reason: 'frame detached' },
        ],
      },
    ),
    // No iframe line: at the end, at the left edge; Playwright's format and full mode list them reversed.
    'partial-frames/no-iframe': tree(
      [{ role: 'button', name: 'Pay', ref: 'e1' }],
      {
        partial: [
          { ref: 'e8', frame: PAY, reason: 'target closed' },
          { ref: 'e9', frame: MAP, reason: 'frame detached' },
        ],
      },
    ),
    // Two for one iframe: in order in Neo's format, reversed in Playwright's format and full mode.
    'partial-frames/two-for-one': tree(
      [
        { role: 'iframe', ref: 'e1' },
        { role: 'button', name: 'Pay', ref: 'e2' },
      ],
      {
        partial: [
          { ref: 'e1', frame: PAY, reason: 'timeout after 2000 ms' },
          { ref: 'e1', frame: MAP, reason: 'frame detached' },
        ],
      },
    ),
    // A frame not read inside a frame read: the comment under the inner iframe, two levels in.
    'partial-frames/nested': tree(
      [
        {
          role: 'iframe',
          ref: 'e1',
          children: [
            { role: 'button', name: 'Next', ref: 'f1e1' },
            { role: 'iframe', ref: 'f1e2' },
          ],
        },
      ],
      {
        partial: [{ ref: 'f1e2', frame: MAP, reason: 'timeout after 2000 ms' }],
      },
    ),
    // A newline in the address: the comment's two lines are one line of the cap, kept or cut whole.
    'partial-frames/newline-address': tree(
      [
        { role: 'button', name: 'Back', ref: 'e1' },
        { role: 'iframe', ref: 'e7' },
      ],
      {
        partial: [
          {
            ref: 'e7',
            frame: `${PAY}\n/form`,
            reason: 'timeout after 2000 ms',
          },
        ],
      },
    ),
    // No node has the ref: Playwright's format and full mode scan for `[ref=` and hang it under text that says it.
    'partial-frames/ref-scan': tree(
      [
        { role: 'paragraph', children: ['Ask at [ref=e7] for help.'] },
        { role: 'button', name: 'Help', ref: 'e1' },
      ],
      { partial: [{ ref: 'e7', frame: PAY, reason: 'target closed' }] },
    ),
    // The ref is a button's: Playwright's format and full mode hang it under the button, Neo's at the end.
    'partial-frames/not-an-iframe': tree(
      [
        { role: 'button', name: 'Pay', ref: 'e7' },
        { role: 'button', name: 'Back', ref: 'e8' },
      ],
      { partial: [{ ref: 'e7', frame: PAY, reason: 'target closed' }] },
    ),
  },
  cases: [
    ...calls('partial-frames/under-iframe', threeFormats()),
    ...calls('partial-frames/no-iframe', threeFormats()),
    ...calls('partial-frames/two-for-one', threeFormats()),
    ...calls('partial-frames/nested', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('partial-frames/newline-address', {
      playwright: playwright(),
      'element-at-limit': element({ maxTokens: 29.75 }),
      'element-one-over': element({ maxTokens: 29.5 }),
      'full-at-limit': full(29.75),
      'full-one-over': full(29.5),
    }),
    ...calls('partial-frames/ref-scan', threeFormats()),
    ...calls('partial-frames/not-an-iframe', threeFormats()),
  ],
}
