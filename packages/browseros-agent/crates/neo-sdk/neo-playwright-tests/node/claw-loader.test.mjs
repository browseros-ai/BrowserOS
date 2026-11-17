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

test('byte replies and the Buffer parameter use Node identity at the Node host boundary', async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'claw-loader-bytes-'))
  const addon = path.join(dir, 'fake-addon.cjs')
  fs.writeFileSync(
    addon,
    `module.exports = {
    async connect() {
      return {
        async openRun() {
          return {
            init: JSON.stringify({ page: { tab: 7 }, pages: [{ tab: 7 }] }),
            async invoke(name) {
              const ok = name === 'page.frames' ? { frames: [] }
                : { $neo: 'bytes', base64: 'iVBORw0KGgo=' };
              return JSON.stringify({ ok });
            },
            async finish() { return JSON.stringify({ status: 'ok' }); },
            log() {},
            async sleep() {},
          };
        },
        async close() {},
      };
    },
  };`,
  )
  process.env.CLAW_NODE_ADDON = addon
  let sdk
  try {
    sdk = await openClaw(0)
    assert.equal(sdk.Buffer, Buffer)
    const bytes = await sdk.page.screenshot()
    assert.ok(Buffer.isBuffer(bytes))
    assert.ok(bytes instanceof Buffer)
    assert.equal(bytes.readUInt32BE(), 0x89504e47)
    assert.ok(bytes.slice(0, 4) instanceof Buffer)
    assert.ok(bytes.subarray(0, 4).equals(Buffer.from('89504e47', 'hex')))
    assert.equal(bytes.toString('base64'), 'iVBORw0KGgo=')
  } finally {
    await sdk?.finish()
    delete process.env.CLAW_NODE_ADDON
    fs.rmSync(dir, { recursive: true, force: true })
  }
})
