// An iframe with fallback markup between its tags (design section 13). The HTML parser keeps
// `<iframe …><p>Your browser does not support iframes.</p></iframe>` as raw text, the engine
// reads it as the iframe node's `text`, and Rust stitches the frame's content under the same
// node. Playwright's renderer prints such a node as one line and drops the frame: on a payment
// page, the card fields. Neo's two formats print the frame and never the markup, so `mode:
// 'full'` keeps every ref the default text prints; Playwright's own YAML (`mode: 'default'`, the
// conformance switch) is untouched. Scripts run through the built prelude against a stand-in
// host; the tree is the ruling's two-iframe page as neo-playwright's prelude_live test reads it
// from a live engine (`FALLBACK_PAGE`): one cross-site card frame twice, the first iframe written
// empty and the second with fallback markup.
import { describe, expect, it } from 'bun:test'
import { renderAriaSnapshotAsYaml } from '../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer'
import { renderSnapshotV1 } from '../src/prelude/snapshot-v1'
import type { SnapshotResult } from '../src/prelude/wire'
import { loadPrelude, SEED } from './support/prelude-runtime'

type Node = SnapshotResult['json'][number]

const FALLBACK = '<p>Your browser does not support iframes.</p>'

/** A card frame's content, refs prefixed by its frame as the engine mints them. */
const card = (frame: string): Node[] => [
  {
    role: 'generic',
    ref: `${frame}e1`,
    children: [
      { role: 'textbox', name: 'Card number', ref: `${frame}e2` },
      { role: 'button', name: 'Pay', ref: `${frame}e3` },
    ],
  },
]

const PLAIN: Node = { role: 'iframe', ref: 'e3', children: card('f1') }
const WITH_FALLBACK: Node = {
  role: 'iframe',
  ref: 'e4',
  text: FALLBACK,
  children: card('f2'),
}

/** The page: the same cross-site frame twice, the second iframe with fallback markup. */
const FRAMES: SnapshotResult = {
  json: [{ role: 'main', ref: 'e2', children: [PLAIN, WITH_FALLBACK] }],
  partial: [],
  document: 1,
}

/** `FRAMES` with the second iframe written empty too: what Neo's formats must print for both. */
const WITHOUT_FALLBACK: SnapshotResult = {
  ...FRAMES,
  json: [
    {
      role: 'main',
      ref: 'e2',
      children: [PLAIN, { role: 'iframe', ref: 'e4', children: card('f2') }],
    },
  ],
}

/** The second iframe's frame was not read: no content, the fallback, a `# partial` comment. */
const UNREAD: SnapshotResult = {
  json: [{ role: 'iframe', ref: 'e4', text: FALLBACK }],
  partial: [
    {
      ref: 'e4',
      frame: 'https://pay.example/card',
      reason: 'timeout after 2000 ms',
    },
  ],
  document: 1,
}

/** The second iframe's node in `result`: what a locator on that iframe gets. */
function secondIframe(result: SnapshotResult): Node {
  const [main] = result.json
  const node = main.children?.[1]
  if (node === undefined || typeof node === 'string')
    throw new Error('the page has a second iframe')
  return node
}

/** A run whose page snapshot answers `result`, and a locator snapshot its second iframe. */
function snapshotting(result: SnapshotResult, upstreamSnapshots = false) {
  return loadPrelude({
    seed: SEED,
    upstreamSnapshots,
    answer: (call) => {
      if (call.name === 'page.ariaSnapshot') return result
      if (call.name === 'locator.ariaSnapshot')
        return { json: [secondIframe(result)], partial: [] }
      return null
    },
  })
}

/** Every ref a text prints. */
const refs = (text: string): Set<string> =>
  new Set([...text.matchAll(/\[ref=([^\]]+)\]/g)].map((match) => match[1]))

/** What `code`'s snapshots print for `result`, each in its own run. */
async function texts(
  result: SnapshotResult,
  code: string,
  upstreamSnapshots = false,
): Promise<string[]> {
  return (await snapshotting(result, upstreamSnapshots).run(code)) as string[]
}

const NEO_FORMS = `return [
  await page.ariaSnapshot({ mode: 'full' }),
  await page.locator('iframe').nth(1).ariaSnapshot({ mode: 'full' }),
  await page.ariaSnapshot(),
  await page.ariaSnapshot({ text: true }),
  await page.ariaSnapshot({ text: false }),
]`

describe("Neo's formats", () => {
  it("tests that mode full prints the second iframe's frame with its refs, on a page and on a locator", async () => {
    const [full, section] = await texts(FRAMES, NEO_FORMS)
    // What the live test's engine printed for the page.
    expect(full).toBe(
      [
        '- main [ref=e2]:',
        '  - iframe [ref=e3]:',
        '    - generic [ref=f1e1]:',
        '      - textbox "Card number" [ref=f1e2]',
        '      - button "Pay" [ref=f1e3]',
        '  - iframe [ref=e4]:',
        '    - generic [ref=f2e1]:',
        '      - textbox "Card number" [ref=f2e2]',
        '      - button "Pay" [ref=f2e3]',
      ].join('\n'),
    )
    expect(section).toBe(
      full
        .split('\n')
        .slice(5)
        .map((line) => line.slice(2))
        .join('\n'),
    )
  })

  it('tests that no form of the default text prints the fallback markup, and each prints what it prints for an iframe written empty', async () => {
    const withFallback = await texts(FRAMES, NEO_FORMS)
    const written = await texts(WITHOUT_FALLBACK, NEO_FORMS)
    expect(withFallback).toEqual(written)
    for (const text of withFallback) expect(text).not.toContain('Your browser')
  })

  it('tests that every ref the default text prints, in either form, is in mode full', async () => {
    const [full, , ...v1] = await texts(FRAMES, NEO_FORMS)
    for (const text of v1)
      for (const ref of refs(text)) expect(refs(full)).toContain(ref)
    expect(refs(full)).toContain('f2e2')
  })

  it('tests that a frame not read prints a bare iframe line and its partial comment, and no fallback markup', async () => {
    const lines = [
      '- iframe [ref=e4]',
      '  # partial: https://pay.example/card not read (timeout after 2000 ms)',
    ].join('\n')
    expect(
      await texts(
        UNREAD,
        `return [
          await page.ariaSnapshot({ mode: 'full' }),
          await page.ariaSnapshot({ text: true }),
          await page.ariaSnapshot({ text: false }),
        ]`,
      ),
    ).toEqual([lines, lines, lines])
  })

  it("tests that fallback markup is no node's text: not a nameless control's own text, and it does not hide a cell's name", () => {
    const json: Node[] = [
      {
        role: 'generic',
        ref: 'e1',
        cursor: 'pointer',
        children: [{ role: 'iframe', ref: 'e2', text: FALLBACK }],
      },
      {
        role: 'cell',
        name: 'Card',
        children: [{ role: 'iframe', ref: 'e3', text: FALLBACK }],
      },
    ]
    expect(
      renderSnapshotV1(
        { json, partial: [] },
        { text: true, urls: false, maxTokens: 8_000 },
      ).text,
    ).toBe(
      [
        '- generic [ref=e1] [cursor=pointer]',
        '- iframe [ref=e2]',
        '- text: Card',
        '- iframe [ref=e3]',
      ].join('\n'),
    )
  })
})

describe("Playwright's YAML", () => {
  it("tests that under the conformance switch and in mode default the text is the vendored renderer's, byte for byte: the fallback line and no frame", async () => {
    const upstream = renderAriaSnapshotAsYaml(FRAMES.json)
    expect(upstream.split('\n').at(-1)).toBe(`  - iframe [ref=e4]: ${FALLBACK}`)
    expect(refs(upstream)).not.toContain('f2e2')
    const section = renderAriaSnapshotAsYaml([secondIframe(FRAMES)])
    expect(
      await texts(
        FRAMES,
        `return [
          await page.ariaSnapshot(),
          await page.ariaSnapshot({ mode: 'ai' }),
          await page.locator('iframe').nth(1).ariaSnapshot({ mode: 'ai' }),
        ]`,
        true,
      ),
    ).toEqual([upstream, upstream, section])
    expect(
      await texts(
        FRAMES,
        `return [
          await page.ariaSnapshot({ mode: 'default' }),
          await page.locator('iframe').nth(1).ariaSnapshot({ mode: 'default' }),
        ]`,
      ),
    ).toEqual([upstream, section])
  })

  it('tests that under the switch a frame not read keeps its fallback line above its partial comment', async () => {
    expect(
      await texts(
        UNREAD,
        "return [await page.ariaSnapshot({ mode: 'ai' })]",
        true,
      ),
    ).toEqual([
      [
        `- iframe [ref=e4]: ${FALLBACK}`,
        '  # partial: https://pay.example/card not read (timeout after 2000 ms)',
      ].join('\n'),
    ])
  })
})
