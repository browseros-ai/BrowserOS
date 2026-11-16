/** Same-request survey and production renderer, against one private browser and local pages. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { openClaw } from './claw-loader.mjs'
import { driveSnapshotRow } from './snapshot-on-top-row.mjs'

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

try {
  for (const row of matrix) {
    const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-on-top-'))
    const sdk = await openClaw(
      Number(process.env.BROWSEROS_CDP_PORT),
      { stateDir, timeoutMs: 120000, hostedMode: true },
      { upstreamSnapshots: false },
    )
    const outputs = []
    let failure
    let page
    try {
      page = await sdk.context.newPage()
      const proof = await driveSnapshotRow(
        page,
        row,
        `http://127.0.0.1:${server.address().port}/${path.basename(row.file)}`,
      )
      outputs.push(...proof.outputs)
    } catch (error) {
      failure = String(error.stack ?? error)
    } finally {
      try {
        await page?.close()
      } finally {
        await sdk.finish()
      }
    }
    // Counts are asserted by prelude_live's table against OpEnv::note. The server's
    // operations.notes column does not hold these debug lines, so never infer counts there.
    console.log(
      JSON.stringify({
        shape: row.id,
        outputs,
        ...(failure ? { failure } : {}),
      }),
    )
    fs.rmSync(stateDir, { recursive: true, force: true })
    if (failure) failures.push({ shape: row.id, failure })
  }
} finally {
  await new Promise((resolve) => server.close(resolve))
}
assert.deepEqual(failures, [], JSON.stringify(failures, null, 2))
console.log(JSON.stringify({ passed: matrix.length }))
