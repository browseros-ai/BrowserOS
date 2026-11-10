/** Neo: Real process tests for independent workers, crashes, interrupted startup and launcher signals. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import test from 'node:test'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'
import { agent } from '../js/prepare.mjs'
import { isAlive, launchBrowser, stopBrowser } from './browser.mjs'

function output(prefix) {
  const root =
    process.env.NEO_NODE_OUTPUT_DIR ||
    path.resolve(agent, '../../.llm/neo-node-results')
  fs.mkdirSync(root, { recursive: true })
  return fs.mkdtempSync(path.join(root, prefix))
}

async function until(predicate) {
  const deadline = Date.now() + 20_000
  while (Date.now() < deadline) {
    const value = predicate()
    if (value) return value
    await sleep(50)
  }
  throw new Error('Timed out waiting for lifecycle evidence')
}

function assertClean(dir) {
  const ownerFile = path.join(dir, 'owner.json')
  if (fs.existsSync(ownerFile)) {
    const owner = JSON.parse(fs.readFileSync(ownerFile))
    if (owner.pid)
      assert.equal(isAlive(-owner.pid), false, `browser group ${owner.pid}`)
    assert.equal(fs.existsSync(owner.profile), false, owner.profile)
    assert.ok(fs.existsSync(path.join(dir, 'cleanup.json')))
  }
  assert.equal(
    fs.readdirSync(dir).some((name) => name.startsWith('profile-')),
    false,
  )
}

function child(command, args, dir, env = process.env) {
  const fd = fs.openSync(path.join(dir, 'launcher.log'), 'wx')
  let proc
  try {
    proc = spawn(command, args, { stdio: ['ignore', fd, fd], env })
  } finally {
    fs.closeSync(fd)
  }
  const exited = new Promise((resolve, reject) => {
    proc.once('error', reject)
    proc.once('exit', (code, signal) => resolve({ code, signal }))
  })
  return { proc, exited }
}

const realBrowser = { skip: !process.env.NEO_E2E_BROWSER, timeout: 90_000 }

test(
  'four workers have independent browsers and port blocks; one crash leaves its peers alive',
  realBrowser,
  async () => {
    const root = output('parallel-lifecycle-')
    const owned = []
    try {
      await Promise.all(
        [0, 1, 2, 3].map(async (slot) => {
          owned[slot] = await launchBrowser(
            process.env.NEO_E2E_BROWSER,
            path.join(root, `worker-${slot}`),
            { slot },
          )
        }),
      )
      assert.equal(new Set(owned.map((browser) => browser.pid)).size, 4)
      assert.equal(new Set(owned.map((browser) => browser.port)).size, 4)
      assert.equal(
        new Set(
          owned.flatMap((browser) =>
            [0, 1, 2, 3].map((offset) => browser.serverPort + offset),
          ),
        ).size,
        16,
      )
      await stopBrowser(owned[0])
      await owned[0].exited
      await owned[0].close()
      for (const peer of owned.slice(1)) assert.ok(isAlive(peer.pid))
      owned[0] = await launchBrowser(
        process.env.NEO_E2E_BROWSER,
        path.join(root, 'worker-4'),
        { slot: 0 },
      )
      for (const peer of owned) assert.ok(isAlive(peer.pid))
    } finally {
      await Promise.all(owned.map((browser) => browser?.close()))
    }
    for (const name of fs.readdirSync(root))
      if (name.startsWith('worker-')) assertClean(path.join(root, name))
    assert.equal(
      fs.readdirSync(root).some((name) => name.startsWith('.browser-slot-')),
      false,
    )
  },
)

test('an unlaunchable executable removes its profile and releases the worker slot', {
  timeout: 30_000,
}, async () => {
  const root = output('failed-start-')
  const dir = path.join(root, 'worker-0')
  await assert.rejects(
    launchBrowser(path.join(root, 'missing-browser'), dir, { slot: 0 }),
    /Browser did not start/,
  )
  assertClean(dir)
  assert.equal(fs.existsSync(path.join(root, '.browser-slot-0')), false)
})

test(
  'worker death during browser startup is cleaned by the guardian',
  realBrowser,
  async () => {
    const root = output('worker-death-')
    const dir = path.join(root, 'worker-0')
    const code = `import { launchBrowser } from ${JSON.stringify(new URL('./browser.mjs', import.meta.url).href)}; await launchBrowser(process.env.NEO_E2E_BROWSER, ${JSON.stringify(dir)}, { slot: 0 });`
    const { proc, exited } = child(
      process.execPath,
      ['--input-type=module', '-e', code],
      root,
    )
    try {
      await until(() => fs.existsSync(path.join(dir, 'owner.json')))
      proc.kill('SIGKILL')
      await exited
      await until(
        () =>
          fs.existsSync(path.join(dir, 'cleanup.json')) &&
          !fs.existsSync(path.join(root, '.browser-slot-0')),
      )
      assertClean(dir)
    } finally {
      if (proc.exitCode === null && proc.signalCode === null)
        proc.kill('SIGKILL')
      await exited
    }
  },
)

for (const [signal, mode, code] of [
  ['SIGINT', 'neo', 130],
  ['SIGTERM', 'reference', 143],
]) {
  test(
    `${signal} of the ${mode} launcher cleans browsers and writes run completion`,
    realBrowser,
    async () => {
      const root = output(`launcher-${mode}-`)
      const args = [
        fileURLToPath(new URL('./run.mjs', import.meta.url)),
        '--skip-build',
        '--workers',
        '2',
        'tests/page/page-click.spec.ts',
        'tests/page/page-evaluate.spec.ts',
      ]
      if (mode === 'reference') args.push('--reference')
      const { proc, exited } = child(process.execPath, args, root, {
        ...process.env,
        NEO_NODE_OUTPUT_DIR: root,
      })
      try {
        const runDir = await until(() => {
          const name = fs
            .readdirSync(root)
            .find((name) => name.startsWith(`${mode}-`))
          if (!name) return undefined
          const dir = path.join(root, name)
          const workers = fs
            .readdirSync(dir)
            .filter((name) => name.startsWith('worker-'))
          return (
            workers.some((name) =>
              fs.existsSync(path.join(dir, name, 'owner.json')),
            ) && dir
          )
        })
        proc.kill(signal)
        assert.equal((await exited).code, code)
        const run = JSON.parse(fs.readFileSync(path.join(runDir, 'run.json')))
        assert.equal(run.signal, signal)
        assert.ok(run.endedAt)
        for (const name of fs.readdirSync(runDir))
          if (name.startsWith('worker-')) assertClean(path.join(runDir, name))
      } finally {
        if (proc.exitCode === null && proc.signalCode === null)
          proc.kill('SIGTERM')
        await exited
      }
    },
  )
}
