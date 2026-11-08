// Returning screenshot Buffers from a run. REAL opendining-2 and fly-unified-3/14 returned
// `await page.screenshot({fullPage:true})` and got `InternalError: out of memory` (a 5.4 MB PNG)
// or a JSON number array; dashdish-3 returned a locator screenshot and got `{"data":[137,80,…]}`.
//
// Inside the script a screenshot is a Buffer in both engines: compared are its length, PNG
// signature, base64 and Node's own JSON of a slice. Leaving the run has no Playwright
// counterpart (a Playwright script hands its Buffer to Node): Neo's outcome keeps each returned
// Uint8Array's place as `{type, bytes}` and its bytes in `binaries`, which the test checks
// against the script's own facts. The browser runs at DPR 2 with a canvas of random pixels, so
// the full page is a PNG well past the 5.4 MB that ran out of memory.
//
// Runs under Node, not Bun, on the shared harness (harness.mjs), through capture.mjs.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { withEngines } from './capture.mjs'
import { skip } from './harness.mjs'

const pages = {
  '/noise.html': `<!doctype html><title>noise</title><style>body{margin:0}</style>
    <button style="width:120px;height:40px">Buy</button>
    <canvas id=noise style="display:block;width:500px;height:1500px"></canvas>
    <div style="height:1500px;background:linear-gradient(#f80,#08f)"></div>
    <script>
      const canvas = document.getElementById('noise')
      canvas.width = 1000
      canvas.height = 3000
      const context = canvas.getContext('2d')
      const data = context.createImageData(1000, 3000)
      for (let i = 0; i < data.data.length; i++)
        data.data[i] = i % 4 === 3 ? 255 : (Math.random() * 256) | 0
      context.putImageData(data, 0, 0)
    </script>`,
}

/** What the script saw of a Buffer, the same in Node and in Neo's realm. */
const FACTS = `const facts = (bytes) => ({
  uint8: bytes instanceof Uint8Array,
  signature: [...bytes.subarray(0, 8)],
  base64: bytes.toString('base64').slice(0, 12),
  json: JSON.stringify(bytes.subarray(0, 4)),
  length: bytes.length,
  height: describeImage(bytes).height,
})`

test('a returned screenshot Buffer leaves the run whole, top level or nested', {
  skip,
  timeout: 180_000,
}, async (t) => {
  await withEngines(
    t,
    { pages, browserArgs: ['--force-device-scale-factor=2'] },
    async (run) => {
      const full = await run(
        '/noise.html',
        `${FACTS}
        const shot = await page.screenshot({ fullPage: true })
        return { shot, facts: facts(shot) }`,
      )
      const reference = full.playwright.value
      const { outcome } = full.neo
      assert.equal(outcome.status, 'ok', JSON.stringify(outcome.error))
      const facts = full.neo.value.facts
      assert.ok(
        reference.facts.length > 5_400_000,
        `${reference.facts.length} bytes`,
      )
      assert.ok(
        facts.length > 5_400_000,
        `past the REAL OOM size: ${facts.length} bytes`,
      )
      // Lengths differ by the noise; everything else of the in-script Buffer matches.
      const { length: _a, ...same } = facts
      const { length: _b, ...referenceSame } = reference.facts
      assert.deepEqual(
        same,
        referenceSame,
        'the in-script Buffer, Neo vs Playwright',
      )
      assert.ok(Buffer.isBuffer(reference.shot), 'Playwright returns a Buffer')
      assert.deepEqual(full.neo.value.shot, {
        type: 'Buffer',
        bytes: facts.length,
      })
      assert.equal(outcome.binaries.length, 1)
      assert.equal(outcome.binaries[0].pointer, '/shot')
      const bytes = Buffer.from(outcome.binaries[0].bytes, 'base64')
      assert.equal(bytes.length, facts.length, 'the bytes the script held')
      assert.deepEqual([...bytes.subarray(0, 8)], facts.signature)

      const nested = await run(
        '/noise.html',
        `${FACTS}
        const element = await page.getByRole('button').first().screenshot()
        return { list: [element, { plain: new Uint8Array([1, 2, 3]) }], title: await page.title(), facts: facts(element) }`,
      )
      const neo = nested.neo
      assert.equal(neo.outcome.status, 'ok', JSON.stringify(neo.outcome.error))
      assert.deepEqual(
        neo.value.facts,
        nested.playwright.value.facts,
        'element Buffer facts',
      )
      assert.equal(neo.value.title, nested.playwright.value.title)
      assert.deepEqual(neo.value.list, [
        { type: 'Buffer', bytes: neo.value.facts.length },
        { plain: { type: 'Uint8Array', bytes: 3 } },
      ])
      assert.deepEqual(
        neo.outcome.binaries.map(({ pointer, bytes }) => [
          pointer,
          Buffer.from(bytes, 'base64').length,
        ]),
        [
          ['/list/0', neo.value.facts.length],
          ['/list/1/plain', 3],
        ],
      )
      assert.deepEqual(
        [...Buffer.from(neo.outcome.binaries[1].bytes, 'base64')],
        [1, 2, 3],
      )
    },
  )
})
