// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This standalone Node harness is not a Turbo task.
/** The loader's setup boundary over a fake add-on; no browser or native build is needed. */
import assert from 'node:assert/strict'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { openClaw } from './claw-loader.mjs'

test('tests that a setup failure stays a marked harness error when cleanup also fails', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'claw-loader-test-'))
  const addon = path.join(dir, 'fake-addon.cjs')
  fs.writeFileSync(
    addon,
    `module.exports = {
  async connect() {
    return {
      async openRun() { throw new Error('connecting to BrowserOS: deadline has elapsed') },
      async close() { throw new Error('trace flush timed out') },
    }
  },
}`,
  )
  process.env.CLAW_NODE_ADDON = addon
  const stateDir = path.join(dir, 'state')
  try {
    await assert.rejects(openClaw(0), (error) => {
      assert.match(
        error.message,
        /^harness-error: Claw setup failed: connecting to BrowserOS: deadline has elapsed; cleanup also failed: trace flush timed out$/,
      )
      assert.match(error.cause.message, /deadline has elapsed/)
      return true
    })
    // A caller-owned state directory is left in place; the default temporary one is removed.
    fs.mkdirSync(stateDir)
    await assert.rejects(openClaw(0, { stateDir }), /^Error: harness-error:/)
    assert.ok(fs.existsSync(stateDir))
  } finally {
    delete process.env.CLAW_NODE_ADDON
    fs.rmSync(dir, { recursive: true, force: true })
  }
})
