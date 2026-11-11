/** Ownership proof stays bounded when unrelated host command lines are very large. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { isAlive, stopBrowser } from './browser.mjs'

test('cleanup survives noisy host listings and refuses a different profile', {
  skip: process.platform === 'win32',
  timeout: 20_000,
}, async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-process-ownership-'))
  const bin = path.join(root, 'bin')
  fs.mkdirSync(bin)
  // This simulates the observed busy-host listing without spawning unrelated processes.
  fs.writeFileSync(
    path.join(bin, 'ps'),
    `#!/usr/bin/env node
const { spawnSync } = require('node:child_process');
const args = process.argv.slice(2);
if (args.includes('-axo') && args.some(arg => arg.includes('command='))) {
  process.stdout.write(('999999 unrelated ' + 'x'.repeat(131072) + '\\n').repeat(16));
} else {
  process.exitCode = spawnSync('/bin/ps', args, { stdio: 'inherit' }).status ?? 1;
}
`,
    { mode: 0o755 },
  )
  const children = []
  const launch = async (profile) => {
    const proc = spawn(
      process.execPath,
      ['-e', 'setInterval(() => {}, 1000)', '--', `--user-data-dir=${profile}`],
      {
        detached: true,
        stdio: 'ignore',
      },
    )
    const exited = new Promise((resolve, reject) => {
      proc.once('error', reject)
      proc.once('exit', resolve)
    })
    children.push({ proc, exited })
    await new Promise((resolve, reject) => {
      proc.once('spawn', resolve)
      proc.once('error', reject)
    })
    return { pid: proc.pid, profile }
  }
  const previousPath = process.env.PATH
  try {
    const owned = await launch(path.join(root, 'profile-owned'))
    const peer = await launch(path.join(root, 'profile-peer'))
    process.env.PATH = `${bin}${path.delimiter}${previousPath}`
    await assert.rejects(
      stopBrowser({ ...peer, profile: owned.profile }),
      /Cannot revalidate browser group/,
    )
    assert.equal(isAlive(peer.pid), true)
    await stopBrowser(owned)
    assert.equal(isAlive(-owned.pid), false)
    assert.equal(
      isAlive(peer.pid),
      true,
      'cleanup must leave the unrelated group alive',
    )
  } finally {
    process.env.PATH = previousPath
    for (const { proc } of children)
      if (proc.exitCode === null && proc.signalCode === null)
        proc.kill('SIGKILL')
    await Promise.all(children.map(({ exited }) => exited))
    fs.rmSync(root, { recursive: true, force: true })
  }
})
