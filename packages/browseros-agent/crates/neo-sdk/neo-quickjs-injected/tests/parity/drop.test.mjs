// locator.drop through Neo and through real Playwright on one BrowserOS neo, each case on two
// fresh tabs serving tests/pages/drop.html (capture.mjs): Playwright's `setupDropzone` and a zone
// that never accepts. Compared: what the script returned or the error's kind, every drag event
// each zone saw (type, point, flags) and what the drop carried (files with name, type, size,
// text; data by type). A path names `at('<file>')`, the same file in each engine's directory.
//
// The script makes its bytes with `bytes()`: Node's `Buffer` for Playwright, a `Uint8Array` in
// Neo's realm, which has no `Buffer` global.
//
// From Playwright's tests/page/page-drop.spec.ts at 26137face: DD-021 to DD-027, plus the
// `position` option and a directory, which Playwright's client refuses.

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

const BYTES = `const bytes = (text) => typeof Buffer === 'function' ? Buffer.from(text) : Uint8Array.from(text, (c) => c.charCodeAt(0))
const settle = (call) => call.then(() => 'resolved', (error) => errorKind(error))
const dropped = () => page.waitForFunction(() => window.__dropInfo).then((handle) => handle.jsonValue())
`

const FILES = { 'hello.txt': 'path-content', 'tree/one.txt': 'one' }

const cases = [
  {
    name: 'DD-021 should drop a file payload',
    source: `await page.locator('#dropzone').drop({ files: { name: 'note.txt', mimeType: 'text/plain', buffer: bytes('hello') } })
      return await dropped()`,
    expect: (value) =>
      assert.deepEqual(value, {
        files: [
          {
            name: 'note.txt',
            type: 'text/plain',
            size: 5,
            text: 'hello',
            dated: true,
          },
        ],
        data: {},
      }),
  },
  {
    name: 'DD-022 should drop multiple file payloads, typed by name without a mimeType',
    source: `await page.locator('#dropzone').drop({ files: [
        { name: 'a.txt', mimeType: 'text/plain', buffer: bytes('AAA') },
        { name: 'b.csv', buffer: bytes('BB') },
      ] })
      return await dropped()`,
    expect: (value) =>
      assert.deepEqual(
        value.files.map((file) => [file.name, file.type, file.text]),
        [
          ['a.txt', 'text/plain', 'AAA'],
          ['b.csv', 'text/csv', 'BB'],
        ],
      ),
  },
  {
    name: 'DD-023 should drop a file by local path',
    source: `await page.locator('#dropzone').drop({ files: at('hello.txt') })
      return await dropped()`,
    expect: (value) =>
      assert.deepEqual(value.files, [
        {
          name: 'hello.txt',
          type: 'text/plain',
          size: 12,
          text: 'path-content',
          dated: true,
        },
      ]),
  },
  {
    name: 'DD-024 should drop clipboard-like data',
    source: `await page.locator('#dropzone').drop({ data: { 'text/plain': 'hello world', 'text/uri-list': 'https://example.com' } })
      return await dropped()`,
    expect: (value) =>
      assert.deepEqual(value, {
        files: [],
        data: {
          'text/plain': 'hello world',
          'text/uri-list': 'https://example.com',
        },
      }),
  },
  {
    name: 'DD-025 should drop files and data together, at a position',
    source: `await page.locator('#dropzone').drop({
        files: { name: 'mix.txt', mimeType: 'text/plain', buffer: bytes('mix') },
        data: { 'text/plain': 'label' },
      }, { position: { x: 15, y: 25 } })
      return await dropped()`,
    expect: (value, facts) => {
      assert.equal(value.files[0].text, 'mix')
      assert.equal(value.data['text/plain'], 'label')
      const drop = facts.events.find((event) => event.type === 'drop')
      assert.deepEqual(
        [drop.x, drop.y],
        [20 + 2 + 15, 20 + 2 + 25],
        'the point is the position',
      )
    },
  },
  {
    name: 'DD-026 should throw when target does not accept drop',
    source: `return await settle(page.locator('#nodrop').drop({ data: { 'text/plain': 'nope' } }))`,
    expect: (value, facts) => {
      assert.deepEqual(value, { error: { name: 'Error', strict: false } })
      assert.deepEqual(
        facts.events.map((event) => event.type),
        ['dragenter', 'dragover', 'dragleave'],
      )
    },
  },
  {
    name: 'DD-027 should throw when neither files nor data provided; a directory and a strict match throw too',
    source: `return {
        empty: await settle(page.locator('#dropzone').drop({})),
        none: await settle(page.locator('#dropzone').drop({ files: [] })),
        directory: await settle(page.locator('#dropzone').drop({ files: at('tree') })),
        strict: await settle(page.locator('div').drop({ data: { 'text/plain': 'x' } })),
      }`,
    expect: (value, facts) => {
      assert.deepEqual(value, {
        empty: { error: { name: 'Error', strict: false } },
        none: { error: { name: 'Error', strict: false } },
        directory: { error: { name: 'Error', strict: false } },
        strict: { error: { name: 'Error', strict: true } },
      })
      assert.deepEqual(facts.events, [], 'nothing was dropped')
    },
  },
]

function seen(engine) {
  return engine.error ? { error: engine.error } : { value: engine.value }
}

test('locator.drop drops the same files and data through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  const page = readFileSync(
    new URL('../pages/drop.html', import.meta.url),
    'utf8',
  )
  await withEngines(t, { pages: { '/drop.html': page } }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(
          '/drop.html',
          BYTES + c.source,
          'window.__facts()',
          FILES,
        )
        t.diagnostic(
          `${c.name}: ${JSON.stringify({ neo: seen(neo), probe: neo.probe })}`,
        )
        assert.deepEqual(seen(neo), seen(playwright), 'what the script saw')
        assert.deepEqual(neo.probe, playwright.probe, 'what the page saw')
        c.expect(playwright.value, playwright.probe)
      })
    }
  })
})
