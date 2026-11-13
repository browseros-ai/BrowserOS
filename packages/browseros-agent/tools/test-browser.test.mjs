/** Exercise the public CLI with real processes, including result and resource ownership. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

const agent = fileURLToPath(new URL('../', import.meta.url))
const cli = fileURLToPath(new URL('./test-browser.mjs', import.meta.url))
const fixture = fileURLToPath(
  new URL('./fixtures/test-browser.mjs', import.meta.url),
)

function run(t, args, overrides = {}, viaBun = false) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'test-browser-check-'))
  const record = path.join(dir, 'browser.json')
  const bin = path.join(dir, 'bin')
  if (overrides.FIXTURE_MASK_COMMAND === '1') {
    fs.mkdirSync(bin)
    fs.copyFileSync(
      fileURLToPath(new URL('./fixtures/test-browser-ps.mjs', import.meta.url)),
      path.join(bin, 'ps'),
    )
    fs.chmodSync(path.join(bin, 'ps'), 0o755)
  }
  const env = {
    ...process.env,
    HOME: dir,
    NEO_E2E_BROWSER: fixture,
    FIXTURE_BROWSER_RECORD: record,
    ...overrides,
    ...(overrides.FIXTURE_MASK_COMMAND === '1'
      ? { PATH: `${bin}${path.delimiter}${process.env.PATH}` }
      : {}),
  }
  const child = spawn(
    viaBun ? 'bun' : process.execPath,
    viaBun ? ['run', 'test:browser', '--', ...args] : [cli, '--', ...args],
    {
      cwd: agent,
      env,
      stdio: ['ignore', 'pipe', 'pipe'],
    },
  )
  let stdout = ''
  let stderr = ''
  child.stdout.on('data', (chunk) => {
    stdout += chunk
  })
  child.stderr.on('data', (chunk) => {
    stderr += chunk
  })
  const exited = new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('close', (code, signal) =>
      resolve({ code, signal, stdout, stderr }),
    )
  })
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null)
      child.kill('SIGTERM')
    await exited
    fs.rmSync(dir, { recursive: true, force: true })
  })
  return {
    child,
    exited,
    dir,
    record,
    stdout: () => stdout,
    stderr: () => stderr,
  }
}

async function until(predicate) {
  const deadline = Date.now() + 10_000
  while (Date.now() < deadline) {
    const value = predicate()
    if (value) return value
    await sleep(25)
  }
  throw new Error('Timed out waiting for launcher evidence')
}

function receipt(running) {
  const root = running.stderr().match(/\[test-browser\] artifacts: (.+)/)?.[1]
  if (!root || !fs.existsSync(path.join(root, 'result.json'))) return undefined
  return JSON.parse(fs.readFileSync(path.join(root, 'result.json'), 'utf8'))
}

async function stopFixture(running) {
  // The fixture deliberately removed its ownership marker; the test itself owns this process.
  if (fs.existsSync(running.record)) {
    const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
    try {
      process.kill(browser.pid, 'SIGKILL')
    } catch (error) {
      if (error.code !== 'ESRCH') throw error
    }
    await until(() => {
      try {
        process.kill(browser.pid, 0)
        return false
      } catch {
        return true
      }
    })
    fs.rmSync(browser.profile, { recursive: true, force: true })
    fs.rmSync(browser.home, { recursive: true, force: true })
  }
}

test('returns the command failure unchanged and removes its private browser', {
  timeout: 20_000,
}, async (t) => {
  const running = run(
    t,
    [process.execPath, fixture, 'command', '+1.98.0', '--', 'name with spaces'],
    {
      FIXTURE_COMMAND_EXIT: '101',
    },
  )
  const result = await running.exited
  assert.equal(result.code, 101, result.stderr)
  const command = JSON.parse(result.stdout)
  const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
  assert.deepEqual(command.args, ['+1.98.0', '--', 'name with spaces'])
  assert.equal(command.cwd, agent.replace(/\/$/, ''))
  assert.equal(command.home, running.dir)
  assert.equal(command.port, browser.port)
  assert.notEqual(browser.home, running.dir)
  assert.ok(browser.args.includes('--disable-browseros-server'))
  assert.equal(fs.existsSync(browser.profile), false)
  assert.equal(fs.existsSync(browser.home), false)
  assert.throws(() => process.kill(browser.pid, 0), { code: 'ESRCH' })
})

test('a failed browser never starts the command', {
  timeout: 20_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_BROWSER_MODE: 'exit',
  })
  const result = await running.exited
  assert.equal(result.code, 125, result.stderr)
  assert.equal(result.stdout, '')
  assert.equal(fs.existsSync(running.record), false)
})

test('the command waits for its own browser to become ready', {
  timeout: 20_000,
}, async (t) => {
  const barrier = path.join(
    os.tmpdir(),
    `browser-ready-${process.pid}-${Date.now()}`,
  )
  t.after(() => fs.rmSync(barrier, { force: true }))
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_BROWSER_READY_FILE: barrier,
  })
  await until(() => fs.existsSync(running.record))
  await sleep(100)
  assert.equal(running.stdout(), '')
  fs.writeFileSync(barrier, '')
  const result = await running.exited
  assert.equal(result.code, 0, result.stderr)
})

test('a completed command cannot leave its child running', {
  timeout: 20_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_COMMAND_CHILD: '1',
  })
  const result = await running.exited
  assert.equal(result.code, 0, result.stderr)
  const command = JSON.parse(result.stdout)
  assert.ok(command.child)
  await until(() => {
    try {
      process.kill(command.child, 0)
      return false
    } catch (error) {
      return error.code === 'ESRCH'
    }
  })
  assert.equal(receipt(running).cleaned, true)
})

for (const [signal, code] of [
  ['SIGINT', 130],
  ['SIGTERM', 143],
  ['SIGHUP', 129],
  ['SIGKILL', null],
]) {
  test(`${signal} of the CLI cleans the running command and browser`, {
    timeout: 20_000,
  }, async (t) => {
    const running = run(t, [process.execPath, fixture, 'command'], {
      FIXTURE_COMMAND_WAIT: '1',
    })
    await until(() => running.stdout().includes('\n'))
    const command = JSON.parse(running.stdout())
    const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
    running.child.kill(signal)
    const result = await running.exited
    assert.equal(result.code, code, result.stderr)
    const completed = await until(
      () => receipt(running)?.state === 'finished' && receipt(running),
    )
    assert.equal(completed.cleaned, true, JSON.stringify(completed))
    assert.equal(fs.existsSync(browser.profile), false)
    assert.equal(fs.existsSync(browser.home), false)
    assert.throws(() => process.kill(browser.pid, 0), { code: 'ESRCH' })
    assert.throws(() => process.kill(completed.command.pid, 0), {
      code: 'ESRCH',
    })
    assert.throws(() => process.kill(command.pid, 0), { code: 'ESRCH' })
  })
}

test('browser death cancels the command and returns failure', {
  timeout: 20_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_COMMAND_WAIT: '1',
  })
  await until(() => running.stdout().includes('\n'))
  const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
  process.kill(browser.pid, 'SIGKILL')
  const result = await running.exited
  assert.equal(result.code, 125, result.stderr)
  assert.equal(receipt(running).cleaned, true)
  assert.throws(() => process.kill(receipt(running).command.pid, 0), {
    code: 'ESRCH',
  })
})

test('parallel commands own independent browsers and finishing one leaves its peer alive', {
  timeout: 20_000,
}, async (t) => {
  const first = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_COMMAND_WAIT: '1',
  })
  const second = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_COMMAND_WAIT: '1',
  })
  await until(
    () => first.stdout().includes('\n') && second.stdout().includes('\n'),
  )
  const a = JSON.parse(fs.readFileSync(first.record, 'utf8'))
  const b = JSON.parse(fs.readFileSync(second.record, 'utf8'))
  assert.notEqual(a.port, b.port)
  assert.notEqual(a.profile, b.profile)
  first.child.kill('SIGTERM')
  assert.equal((await first.exited).code, 143)
  assert.doesNotThrow(() => process.kill(b.pid, 0))
  second.child.kill('SIGTERM')
  assert.equal((await second.exited).code, 143)
})

test('the Bun command preserves its child failure code', {
  timeout: 20_000,
}, async (t) => {
  const running = run(
    t,
    [process.execPath, fixture, 'command'],
    { FIXTURE_COMMAND_EXIT: '101' },
    true,
  )
  const result = await running.exited
  assert.equal(result.code, 101, result.stderr)
})

test('a hard-killed CLI during startup still cleans its browser', {
  timeout: 20_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_BROWSER_READY_FILE: path.join(
      os.tmpdir(),
      `never-ready-${process.pid}-${Date.now()}`,
    ),
  })
  await until(() => fs.existsSync(running.record))
  const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
  running.child.kill('SIGKILL')
  await running.exited
  assert.equal(running.stdout(), '')
  assert.equal(fs.existsSync(browser.profile), false)
  assert.equal(fs.existsSync(browser.home), false)
  assert.throws(() => process.kill(browser.pid, 0), { code: 'ESRCH' })
})

test('unproven cleanup returns failure and retains the live browser state', {
  timeout: 20_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_BROWSER_MODE: 'unowned',
  })
  try {
    const result = await running.exited
    assert.equal(result.code, 125, result.stderr)
    assert.equal(receipt(running).cleaned, false)
    const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
    assert.equal(fs.existsSync(browser.profile), true)
    assert.equal(fs.existsSync(browser.home), true)
  } finally {
    await stopFixture(running)
  }
})

test('cancellation reports failed cleanup during browser startup', {
  timeout: 20_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_BROWSER_MODE: 'unowned',
    FIXTURE_BROWSER_READY_FILE: path.join(
      os.tmpdir(),
      `unowned-startup-${process.pid}-${Date.now()}`,
    ),
  })
  try {
    await until(() => fs.existsSync(running.record))
    running.child.kill('SIGTERM')
    const result = await running.exited
    assert.equal(result.code, 143, result.stderr)
    assert.equal(receipt(running).cleaned, false)
    assert.equal(running.stdout(), '')
  } finally {
    await stopFixture(running)
  }
})

test('an unexpected owner death still drains its recorded groups and temporary state', {
  timeout: 20_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_COMMAND_WAIT: '1',
  })
  await until(() => running.stdout().includes('\n'))
  const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
  process.kill(receipt(running).owner, 'SIGKILL')
  const result = await running.exited
  assert.equal(result.code, 125, result.stderr)
  assert.equal(fs.existsSync(browser.profile), false)
  assert.equal(fs.existsSync(browser.home), false)
  assert.equal(receipt(running).cleaned, true)
})

test('a browser that never publishes readiness fails at the startup deadline', {
  timeout: 45_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_BROWSER_READY_FILE: path.join(
      os.tmpdir(),
      `startup-deadline-${process.pid}-${Date.now()}`,
    ),
  })
  const result = await running.exited
  assert.equal(result.code, 125, result.stderr)
  assert.match(result.stderr, /within 30 seconds/)
  assert.equal(result.stdout, '')
  const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
  assert.equal(fs.existsSync(browser.profile), false)
  assert.equal(fs.existsSync(browser.home), false)
  assert.equal(receipt(running).cleaned, true)
})

test('failed command-group ownership stops the browser but preserves its state', {
  timeout: 20_000,
}, async (t) => {
  const running = run(t, [process.execPath, fixture, 'command'], {
    FIXTURE_MASK_COMMAND: '1',
  })
  try {
    const result = await running.exited
    assert.equal(result.code, 125, result.stderr)
    const completed = receipt(running)
    assert.equal(completed.cleaned, false)
    assert.match(
      completed.cleanupErrors.join(' '),
      /Cannot revalidate command group/,
    )
    const browser = JSON.parse(fs.readFileSync(running.record, 'utf8'))
    assert.throws(() => process.kill(browser.pid, 0), { code: 'ESRCH' })
    assert.equal(fs.existsSync(browser.profile), true)
    assert.equal(fs.existsSync(browser.home), true)
  } finally {
    await stopFixture(running)
  }
})
