/** Same-request survey and production renderer, against one private browser and local pages. */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { openClaw } from './claw-loader.mjs'

const folder = fileURLToPath(new URL('./snapshot-on-top/', import.meta.url))
let matrix = JSON.parse(
  fs.readFileSync(path.join(folder, 'matrix.json'), 'utf8'),
)
const selected = process.argv.slice(2)
if (selected.length) {
  matrix = matrix.filter((row) => selected.includes(row.id))
  assert.equal(
    matrix.length,
    selected.length,
    'every selected fixture must exist',
  )
}
const server = http.createServer((req, res) => {
  const name = path.basename(new URL(req.url, 'http://fixture').pathname)
  const file = path.join(folder, 'pages', name)
  if (!fs.existsSync(file)) {
    res.writeHead(404)
    res.end()
    return
  }
  res.setHeader('content-type', 'text/html; charset=utf-8')
  res.end(fs.readFileSync(file))
})
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
const failures = []

function checkNotes(text, notes) {
  const lines = text.split('\n').filter((line) => line.startsWith('# on top: '))
  assert.equal(lines.length, notes.length, text)
  for (const [index, note] of notes.entries()) {
    const prefix = `# on top: ${note.role}${note.name ? ` ${JSON.stringify(note.name)}` : ''} [ref=`
    assert.ok(lines[index].startsWith(prefix), `${prefix}\n${text}`)
    assert.ok(
      lines[index].endsWith(
        note.extent === 'whole'
          ? ' over the whole view'
          : ' over part of the view',
      ),
      text,
    )
    for (const flag of note.flags)
      assert.ok(lines[index].includes(`, ${flag}`), text)
    for (const flag of ['blank', 'loading'])
      if (!note.flags.includes(flag))
        assert.ok(!lines[index].includes(`, ${flag}`), text)
    if (note.says)
      assert.ok(
        lines[index].includes(`, says ${JSON.stringify(note.says)}`),
        text,
      )
    const following =
      text.split('\n')[text.split('\n').indexOf(lines[index]) + 1] ?? ''
    if (note.controls.length) {
      assert.ok(following.startsWith('#   its controls: '), text)
      for (const name of note.controls.slice(0, 4))
        assert.ok(following.includes(JSON.stringify(name)), text)
    } else assert.ok(!following.startsWith('#   its controls: '), text)
  }
}

try {
  for (const row of matrix) {
    const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-on-top-'))
    const sdk = await openClaw(
      Number(process.env.BROWSEROS_CDP_PORT),
      { stateDir, timeoutMs: 120000, hostedMode: true },
      { upstreamSnapshots: false },
    )
    const run = sdk.native.id
    const expectedCounts = []
    const outputs = []
    let failure
    let page
    try {
      page = await sdk.context.newPage()
      await page.setViewportSize(row.viewport)
      await page.goto(
        `http://127.0.0.1:${server.address().port}/${path.basename(row.file)}`,
      )
      await page.waitForFunction(() => window.fixtureReady === true)
      const original = await page.ariaSnapshot({ mode: 'default' })
      const snap = async (count, options = {}) => {
        const text = await page.ariaSnapshot(options)
        expectedCounts.push(count)
        outputs.push(text)
        return text
      }
      const initial = await snap(row.expectedControls)
      checkNotes(initial, row.expectedNotes)
      if (row.firstText) {
        assert.ok(initial.indexOf(row.firstText) >= 0, initial)
        assert.ok(
          initial.indexOf(row.firstText) < initial.indexOf('Outside box'),
          initial,
        )
        assert.equal(initial.split(row.firstText).length - 1, 1, initial)
      }
      if (row.longPortal) {
        assert.ok(
          initial.startsWith('# on top: dialog "Log in or sign up"'),
          initial,
        )
        assert.ok(initial.slice(0, 400).includes('button "Close"'), initial)
        assert.ok(
          initial.indexOf('- dialog "Log in or sign up"') <
            initial.indexOf('Reserve Tickets'),
          initial,
        )
      }
      const full = await snap(row.expectedControls, { mode: 'full' })
      checkNotes(full, row.expectedNotes)
      const json = await page.ariaSnapshotJSON({ mode: 'ai' })
      assert.ok(
        !JSON.stringify(json).includes('"onTop":'),
        'JSON mode must not carry Neo content marks',
      )
      assert.equal(
        await page.ariaSnapshot({ mode: 'default' }),
        original,
        'upstream mode must be byte-identical',
      )
      for (const maxTokens of [1, 8, 80])
        checkNotes(
          await snap(row.expectedControls, { maxTokens }),
          row.expectedNotes,
        )
      for (const phase of row.phases ?? []) {
        if (phase.after.startsWith('scroll')) {
          const y = Number(/y=(\d+)/.exec(phase.after)?.[1] ?? 0)
          await page.evaluate((y) => window.scrollTo(0, y), y)
        } else if (phase.after.includes('close')) {
          const name = row.expectedNotes
            .flatMap((note) => note.controls)
            .find((name) => /close/i.test(name))
          assert.ok(name, 'a close phase must name its actual control')
          await page.getByRole('button', { name, exact: true }).click()
        }
        checkNotes(await snap(phase.expectedControls), phase.notes)
      }
    } catch (error) {
      failure = String(error.stack ?? error)
    } finally {
      try {
        await page?.close()
      } finally {
        await sdk.finish()
      }
    }
    try {
      // Read the actual operation trace after close flushed it; no second detector or DOM count.
      const query = spawnSync(
        'python3',
        [
          '-c',
          `
import json,sqlite3,sys
with sqlite3.connect(sys.argv[1]) as db:
 rows=db.execute('select notes from operations where run_id=? and name=? order by seq',(sys.argv[2],'page.ariaSnapshot')).fetchall()
print(json.dumps([note for (notes,) in rows for note in json.loads(notes or '[]') if 'cover survey:' in note]))
`,
          path.join(stateDir, 'claw-sdk', 'trace.db'),
          run,
        ],
        { encoding: 'utf8' },
      )
      assert.equal(query.status, 0, query.stderr)
      const measurements = JSON.parse(query.stdout).map((line) => {
        const match = /cover survey: (\d+) controls tested in ([\d.]+) ms/.exec(
          line,
        )
        assert.ok(match, line)
        return { tested: Number(match[1]), elapsedMs: Number(match[2]) }
      })
      assert.deepEqual(
        measurements.map((measure) => measure.tested),
        expectedCounts,
        'each snapshot must test the row count, never pass on an empty set',
      )
      console.log(
        JSON.stringify({
          shape: row.id,
          measurements,
          outputs,
          ...(failure ? { failure } : {}),
        }),
      )
    } catch (error) {
      failure = `${failure ?? ''}\n${error.stack ?? error}`
    } finally {
      fs.rmSync(stateDir, { recursive: true, force: true })
    }
    if (failure) failures.push({ shape: row.id, failure })
  }
} finally {
  await new Promise((resolve) => server.close(resolve))
}
assert.deepEqual(failures, [], JSON.stringify(failures, null, 2))
console.log(JSON.stringify({ passed: matrix.length }))
