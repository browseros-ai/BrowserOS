// locator/page/frame.setInputFiles through Neo and through real Playwright on one BrowserOS neo,
// each case on two fresh tabs serving tests/pages/upload.html (capture.mjs). Compared: what the
// script returned or the error's kind, then every `input` and `change` event and every file
// each input holds (name, type, size, text, relative path). A path names `at('<file>')`: in a
// temporary directory for Playwright, inside the run's artifact directory for Neo, both
// holding the same files.
//
// The script makes its bytes with `bytes()`: Node's `Buffer` for Playwright, whose client
// requires one, and a `Uint8Array` in Neo's realm, which has no `Buffer` global.
//
// From Playwright's tests/page/page-set-input-files.spec.ts at 26137face: 'IF-002 should upload
// a folder', 'IF-004 ... throw for multiple directories', 'IF-005 should throw if a directory
// and files are passed', 'IF-006 ... a folder in a normal file upload input', 'IF-007 ... a file
// in a directory upload input', 'IF-010 should throw an error if the file does not exist',
// 'IF-013 should work', 'IF-014 should set from memory', 'IF-016 should detect mime type' and
// 'IF-018 should emit input and change events'.

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

const BYTES = `const bytes = (text) => typeof Buffer === 'function' ? Buffer.from(text) : Uint8Array.from(text, (c) => c.charCodeAt(0))
const settle = (call) => call.then(() => 'resolved', (error) => errorKind(error))
`

const FILES = {
  'uploads/a.txt': 'alpha',
  'uploads/b.json': '{"b":1}',
  'uploads/page.html': '<p>page</p>',
  'tree/one.txt': 'one',
  'tree/nested/two.txt': 'two',
  'other/three.txt': 'three',
}

const cases = [
  {
    name: 'IF-014 / IF-018 a buffer from memory, with its input and change events',
    source: `return await settle(page.locator('#single').setInputFiles({ name: 'test.txt', mimeType: 'text/plain', buffer: bytes('this is a test') }))`,
    expect: (facts) =>
      assert.deepEqual(facts.single, [
        {
          name: 'test.txt',
          type: 'text/plain',
          size: 14,
          text: 'this is a test',
          path: '',
        },
      ]),
  },
  {
    name: 'IF-016 buffers without a mimeType are typed by their names, as many as a multiple input takes',
    source: `return await settle(page.setInputFiles('#multi', [
      { name: 'data.json', buffer: bytes('{}') },
      { name: 'report.docx', buffer: bytes('doc') },
      { name: 'noext', buffer: bytes('?') },
    ]))`,
    expect: (facts) =>
      assert.deepEqual(
        facts.multi.map((file) => file.type),
        [
          'application/json',
          'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
          'application/octet-stream',
        ],
      ),
  },
  {
    name: 'an empty list clears the files, through a label to its input',
    source: `await page.mainFrame().setInputFiles('#label', { name: 'x.txt', mimeType: 'text/plain', buffer: bytes('x') })
      return await settle(page.locator('#label').setInputFiles([]))`,
    expect: (facts) => assert.deepEqual(facts.single, []),
  },
  {
    name: 'IF-013 files by path, one and several',
    source: `await page.locator('#single').setInputFiles(at('uploads/a.txt'))
      return await settle(page.locator('#multi').setInputFiles([at('uploads/a.txt'), at('uploads/b.json'), at('uploads/page.html')]))`,
    expect: (facts) => {
      assert.deepEqual(
        facts.single.map((file) => [file.name, file.text]),
        [['a.txt', 'alpha']],
      )
      assert.deepEqual(
        facts.multi.map((file) => file.name),
        ['a.txt', 'b.json', 'page.html'],
      )
    },
  },
  {
    name: 'IF-002 a folder into a webkitdirectory input, after its input event',
    source: `return await settle(page.locator('#folder').setInputFiles(at('tree')))`,
    expect: (facts) =>
      assert.deepEqual(
        facts.folder.map((file) => [file.path, file.text]),
        [
          ['tree/nested/two.txt', 'two'],
          ['tree/one.txt', 'one'],
        ],
      ),
  },
  {
    name: 'IF-004 / IF-005 / IF-006 / IF-007 / IF-010 and the kind errors, each before any file is set',
    source: `const results = {}
      for (const [name, call] of [
        ['two folders', () => page.locator('#folder').setInputFiles([at('tree'), at('other')])],
        ['folder and file', () => page.locator('#folder').setInputFiles([at('tree'), at('uploads/a.txt')])],
        ['folder into a file input', () => page.locator('#single').setInputFiles(at('tree'))],
        ['file into a folder input', () => page.locator('#folder').setInputFiles(at('uploads/a.txt'))],
        ['missing file', () => page.locator('#single').setInputFiles(at('uploads/missing.txt'))],
        ['mixed paths and buffers', () => page.locator('#single').setInputFiles([at('uploads/a.txt'), { name: 'b', mimeType: 'text/plain', buffer: bytes('b') }])],
        ['not an input', () => page.locator('#plain').setInputFiles({ name: 'p', mimeType: 'text/plain', buffer: bytes('p') })],
        ['two files into a single input', () => page.locator('#single').setInputFiles([{ name: 'a', mimeType: 'text/plain', buffer: bytes('a') }, { name: 'b', mimeType: 'text/plain', buffer: bytes('b') }])],
        ['strict', () => page.locator('input').setInputFiles([])],
      ]) results[name] = await settle(call())
      return results`,
    expect: (facts, value) => {
      for (const [name, result] of Object.entries(value))
        assert.equal(result.error?.name, 'Error', name)
      assert.equal(value.strict.error.strict, true)
      assert.deepEqual(facts.events, [], 'nothing was set')
    },
  },
  {
    name: 'a buffer into a text input resolves and sets nothing; a path there times out',
    source: `return {
        buffer: await settle(page.locator('#text').setInputFiles({ name: 't', mimeType: 'text/plain', buffer: bytes('t') })),
        path: await settle(page.locator('#text').setInputFiles(at('uploads/a.txt'), { timeout: 500 })),
      }`,
    expect: (facts, value) => {
      assert.equal(value.buffer, 'resolved')
      assert.equal(value.path.error.name, 'TimeoutError')
      assert.deepEqual(facts.events, [])
    },
  },
]

function seen(engine) {
  return engine.error ? { error: engine.error } : { value: engine.value }
}

test('setInputFiles sets the same files through Neo and Playwright', {
  skip,
  timeout: 600_000,
}, async (t) => {
  const upload = readFileSync(
    new URL('../pages/upload.html', import.meta.url),
    'utf8',
  )
  await withEngines(t, { pages: { '/upload.html': upload } }, async (run) => {
    for (const c of cases) {
      await t.test(c.name, async () => {
        const { playwright, neo } = await run(
          '/upload.html',
          BYTES + c.source,
          'window.__facts()',
          FILES,
        )
        t.diagnostic(
          `${c.name}: ${JSON.stringify({ neo: seen(neo), probe: neo.probe })}`,
        )
        assert.deepEqual(seen(neo), seen(playwright), 'what the script saw')
        assert.deepEqual(neo.probe, playwright.probe, 'what the page saw')
        c.expect(playwright.probe, playwright.value)
      })
    }
  })
})
