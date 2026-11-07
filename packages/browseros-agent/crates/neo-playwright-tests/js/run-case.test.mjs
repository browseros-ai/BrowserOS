/** Offline lifecycle faults use private child processes and sockets, never upstream API stand-ins. */
import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { EventEmitter } from 'node:events'
import fs from 'node:fs'
import net from 'node:net'
import os from 'node:os'
import path from 'node:path'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'
import { Worker } from 'node:worker_threads'
import {
  CaseCancellation,
  runCase,
  runner,
  validateRunnerExit,
} from './run-case.mjs'

const modulePath = fileURLToPath(new URL('./run-case.mjs', import.meta.url))
const ioPath = new URL('./io-worker.mjs', import.meta.url)
// Bun 1.3.6 keeps a native-I/O Worker alive after MessagePort.close even after
// the child is reaped. Production uses Node, whose shutdown assertions remain strict.
const ioTest = process.versions.bun ? test.skip : test

const entry = { key: 'offline-process-fault', name: 'offline-process-fault' }
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
async function until(predicate, message, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (!predicate()) {
    assert(Date.now() < deadline, message)
    await pause(10)
  }
}
function temporary(t) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-case-fault-'))
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }))
  return dir
}
function scope(t) {
  const host = Object.assign(new EventEmitter(), { ppid: process.ppid })
  const cancellation = new CaseCancellation({ host, parentPollMs: 10 })
  t.after(() => cancellation.dispose())
  return { host, cancellation }
}
const outcome = (kind) => ({
  kind,
  exit: { code: 0, signal: null },
  runner: { status: 'passed', outcomes: [], errors: [] },
  journal: { calls: [] },
  infrastructureErrors: [],
})
const browser = (overrides = {}) => ({
  endpoint: 'ws://127.0.0.1:1/private-fault-only',
  pageId: 'offline',
  version: {},
  snapshot: async () => ({ status: 'complete', pages: [], events: [] }),
  close: async () => {},
  ...overrides,
})
const settings = (outputDir) => ({
  outputDir,
  entry,
  overlay: outputDir,
  executable: 'unused-fault-fixture',
  worker: 'unused-fault-fixture',
})
function alive(pid) {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    if (error.code === 'ESRCH') return false
    throw error
  }
}
async function listen(port) {
  const server = net.createServer()
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(port, '127.0.0.1', resolve)
  })
  return server
}
const closeServer = (server) => new Promise((resolve) => server.close(resolve))

async function runnerFixture(
  t,
  {
    report = 'passed',
    code = 0,
    signal = null,
    hold = false,
    ignore = false,
    descendant = false,
  } = {},
) {
  const dir = temporary(t)
  const cli = path.join(dir, 'fault-cli.mjs')
  const descendantCode = `process.on('SIGINT',()=>{});require('node:fs').writeFileSync(${JSON.stringify(path.join(dir, 'descendant.pid'))},String(process.pid));setInterval(()=>{},1000)`
  const childStart = descendant
    ? `spawn(process.execPath, ['-e', ${JSON.stringify(descendantCode)}], {stdio:'ignore'});`
    : ''
  fs.writeFileSync(
    cli,
    `
    import fs from 'node:fs';
    import {spawn} from 'node:child_process';
    ${childStart}
    const outcome = ${JSON.stringify({ status: report, outcomes: [{ key: entry.key, rawStatus: report === 'failed' ? 'failed' : 'passed', expectedStatus: 'passed', errors: [] }], errors: [] })};
    fs.writeFileSync(process.env.NEO_TESTS_RESULT, JSON.stringify(outcome));
    fs.writeFileSync(process.env.NEO_TESTS_JOURNAL, JSON.stringify({calls:[]}));
    ${ignore ? "process.on('SIGINT', () => {});" : "process.on('SIGINT', () => process.exit(0));"}
    fs.writeFileSync(${JSON.stringify(path.join(dir, 'ready'))}, String(process.pid));
    ${hold ? 'setInterval(() => {}, 1000);' : signal ? `process.kill(process.pid, ${JSON.stringify(signal)});` : `process.exit(${code});`}
  `,
  )
  const invoke = (signal, options = {}) =>
    runner(
      {
        ...settings(dir),
        outputDir: dir,
        port: 17001,
        kind: 'reference',
        engine: path.join(dir, 'engine.json'),
        signal,
      },
      {
        cli,
        timeoutMs: 30_000,
        interruptMs: 40,
        killMs: 500,
        groupGraceMs: 20,
        ...options,
      },
    )
  return { dir, invoke }
}

test('runner rejects a signal or contradictory exit after passing artifacts were written', async (t) => {
  for (const failure of [
    { code: 42 },
    { signal: 'SIGTERM' },
    { report: 'unknown' },
  ]) {
    const fixture = await runnerFixture(t, failure)
    const phase = await fixture.invoke()
    assert.equal(phase.runner.status, failure.report || 'passed')
    assert(phase.journal)
    assert(
      phase.infrastructureErrors.some((error) =>
        /did not exit normally|contradicts|Unknown/.test(error),
      ),
    )
  }
})

test('ordinary and expected assertion failures keep legitimate runner exit combinations', async (t) => {
  const failed = await runnerFixture(t, { report: 'failed', code: 1 })
  assert.deepEqual((await failed.invoke()).infrastructureErrors, [])
  const passed = await runnerFixture(t)
  assert.deepEqual((await passed.invoke()).infrastructureErrors, [])
  assert.deepEqual(
    validateRunnerExit(
      { code: 0, signal: null },
      {
        status: 'passed',
        outcomes: [{ rawStatus: 'failed', expectedStatus: 'failed' }],
      },
    ),
    [],
  )
  assert.deepEqual(
    validateRunnerExit(
      { code: 0, signal: null },
      {
        status: 'passed',
        outcomes: [{ rawStatus: 'skipped', expectedStatus: 'skipped' }],
      },
    ),
    [],
  )
})

test('runner cancellation escalates an ignored interrupt and reaps its owned group', async (t) => {
  const fixture = await runnerFixture(t, { hold: true, ignore: true })
  const controller = new AbortController()
  const pending = fixture.invoke(controller.signal)
  await until(
    () => fs.existsSync(path.join(fixture.dir, 'ready')),
    'fault runner did not start',
  )
  const pid = Number(fs.readFileSync(path.join(fixture.dir, 'ready'), 'utf8'))
  t.after(() => {
    if (alive(pid)) process.kill(-pid, 'SIGKILL')
  })
  controller.abort(new Error('offline cancel'))
  const phase = await pending
  assert.deepEqual(phase.exit, { code: null, signal: 'SIGKILL' })
  assert(
    phase.infrastructureErrors.some((error) =>
      error.includes('offline cancel'),
    ),
  )
  assert.equal(alive(pid), false)
})

test('runner group cancellation also reaps descendants and preserves an unrelated group', async (t) => {
  const fixture = await runnerFixture(t, {
    hold: true,
    ignore: true,
    descendant: true,
  })
  const unrelated = spawn(
    process.execPath,
    ['-e', 'setInterval(() => {}, 1000)'],
    { detached: true, stdio: 'ignore' },
  )
  const unrelatedExit = new Promise((resolve) =>
    unrelated.once('exit', resolve),
  )
  t.after(async () => {
    unrelated.kill('SIGTERM')
    await unrelatedExit
  })
  const controller = new AbortController()
  const pending = fixture.invoke(controller.signal)
  const file = path.join(fixture.dir, 'descendant.pid')
  await until(
    () => fs.existsSync(file) && fs.existsSync(path.join(fixture.dir, 'ready')),
    'runner descendant did not start',
  )
  const pid = Number(fs.readFileSync(file, 'utf8'))
  t.after(() => {
    if (alive(pid)) process.kill(pid, 'SIGKILL')
  })
  controller.abort(new Error('cancel complete owned group'))
  const phase = await pending
  assert.equal(phase.exit.signal, 'SIGKILL')
  await until(() => !alive(pid), 'runner descendant survived group kill')
  assert.equal(alive(unrelated.pid), true)
})

test('the real lease binds nonzero ports and retains one origin block across sequential phases', async (t) => {
  const dir = temporary(t)
  const { cancellation } = scope(t)
  const ports = []
  const pair = await runCase(settings(dir), {
    cancellation,
    startBrowser: async () => browser(),
    runRunner: async ({ kind, port }) => {
      ports.push(port)
      const servers = []
      try {
        for (let i = 0; i < 4; i++) servers.push(await listen(port + i))
      } finally {
        await Promise.all(servers.map(closeServer))
      }
      return outcome(kind)
    },
  })
  assert(ports[0] > 0)
  assert.deepEqual(ports, [pair.port, pair.port])
  assert.equal(
    fs.existsSync(
      path.join(os.tmpdir(), 'neo-playwright-tests-ports', String(pair.port)),
    ),
    false,
  )
  assert.equal(pair.cancellation, null)
})

test('an already cancelled case starts no phase and still publishes both incomplete records', async (t) => {
  const dir = temporary(t)
  const { host, cancellation } = scope(t)
  host.emit('SIGINT')
  let starts = 0
  const pair = await runCase(settings(dir), {
    cancellation,
    startBrowser: async () => {
      starts++
      return browser()
    },
  })
  assert.equal(starts, 0)
  assert.match(pair.cancellation, /SIGINT/)
  assert.deepEqual(Object.keys(pair.phases), ['reference', 'neo'])
  assert.deepEqual(
    JSON.parse(fs.readFileSync(path.join(dir, 'pair.json'), 'utf8')),
    pair,
  )
  for (const name of ['SIGINT', 'SIGTERM', 'SIGHUP'])
    assert.equal(host.listenerCount(name), 0)
})

test('the authoritative parent identity rejects an already orphaned case before setup', async (t) => {
  const dir = temporary(t)
  const host = Object.assign(new EventEmitter(), { ppid: 1 })
  const cancellation = new CaseCancellation({
    host,
    parentPid: 12345,
    parentPollMs: 10,
  })
  let starts = 0
  const pair = await runCase(settings(dir), {
    cancellation,
    startBrowser: async () => {
      starts++
      return browser()
    },
  })
  assert.equal(starts, 0)
  assert.match(pair.cancellation, /parent exited before startup/)
  assert.equal(pair.port, null)
})

test('cancellation during port setup releases the acquired lease without starting a browser', async (t) => {
  const dir = temporary(t)
  const { host, cancellation } = scope(t)
  let released = 0
  let starts = 0
  const pair = await runCase(settings(dir), {
    cancellation,
    lease: async () => {
      host.emit('SIGTERM')
      return { port: 19001, release: () => released++ }
    },
    startBrowser: async () => {
      starts++
      return browser()
    },
  })
  assert.equal(starts, 0)
  assert.equal(released, 1)
  assert.match(pair.cancellation, /SIGTERM/)
})

test('startup receives the case token and no runner or second phase starts after SIGHUP', async (t) => {
  const dir = temporary(t)
  const { host, cancellation } = scope(t)
  let starts = 0
  let runs = 0
  let closed = false
  const pair = await runCase(settings(dir), {
    cancellation,
    startBrowser: async ({ signal }) => {
      starts++
      const server = await listen(0)
      const aborted = new Promise((_, reject) =>
        signal.addEventListener(
          'abort',
          async () => {
            await closeServer(server)
            closed = true
            reject(signal.reason)
          },
          { once: true },
        ),
      )
      host.emit('SIGHUP')
      return aborted
    },
    runRunner: async () => {
      runs++
    },
  })
  assert.equal(starts, 1)
  assert.equal(runs, 0)
  assert.equal(closed, true)
  assert.match(pair.cancellation, /SIGHUP/)
  assert.equal(pair.phases.neo.runner, null)
})

test('interrupting the reference runner prevents Neo and completes browser cleanup', async (t) => {
  const dir = temporary(t)
  const fixture = await runnerFixture(t, { hold: true })
  const { host, cancellation } = scope(t)
  let starts = 0
  let closed = 0
  const pair = await runCase(settings(dir), {
    cancellation,
    startBrowser: async () => {
      starts++
      return browser({
        close: async () => {
          closed++
        },
      })
    },
    runRunner: async ({ signal }) => {
      const pending = fixture.invoke(signal)
      await until(
        () => fs.existsSync(path.join(fixture.dir, 'ready')),
        'reference runner did not start',
      )
      host.emit('SIGTERM')
      return pending
    },
  })
  assert.equal(starts, 1)
  assert.equal(closed, 1)
  assert.equal(pair.phases.neo.runner, null)
  assert.match(pair.cancellation, /SIGTERM/)
})

test('observation and close remain inside cancellation ownership', async (t) => {
  for (const during of ['snapshot', 'close']) {
    const dir = temporary(t)
    const { host, cancellation } = scope(t)
    let starts = 0
    let closes = 0
    const pair = await runCase(settings(dir), {
      cancellation,
      startBrowser: async () => {
        starts++
        return browser({
          snapshot: async () => {
            if (during === 'snapshot') {
              setTimeout(() => host.emit('SIGINT'), 10)
              return new Promise(() => {})
            }
            return { status: 'complete', pages: [] }
          },
          close: async () => {
            closes++
            if (during === 'close') host.emit('SIGHUP')
          },
        })
      },
      runRunner: async ({ kind }) => outcome(kind),
    })
    assert.equal(starts, 1)
    assert.equal(closes, 1)
    assert(pair.cancellation)
    assert.equal(pair.phases.neo.runner, null)
  }
})

test('cleanup failure taints phase evidence and lease cleanup remains independent', async (t) => {
  const dir = temporary(t)
  const { cancellation } = scope(t)
  let released = 0
  const pair = await runCase(settings(dir), {
    cancellation,
    lease: async () => ({
      port: 19001,
      release: () => {
        released++
        throw new Error('lease fault')
      },
    }),
    startBrowser: async () =>
      browser({
        close: async () => {
          throw new Error('browser close fault')
        },
      }),
    runRunner: async ({ kind }) => outcome(kind),
  })
  assert.equal(released, 1)
  for (const phase of Object.values(pair.phases)) {
    assert(
      phase.infrastructureErrors.some((error) =>
        error.includes('browser close fault'),
      ),
    )
    assert(
      phase.infrastructureErrors.some((error) => error.includes('lease fault')),
    )
  }
})

async function ioFixture(t, mode) {
  const dir = temporary(t)
  const executable = path.join(dir, 'native-fault.cjs')
  const pidFile = path.join(dir, 'native.pid')
  const eofFile = path.join(dir, 'native.eof')
  fs.writeFileSync(
    executable,
    `#!/usr/bin/env node
    const fs = require('node:fs');
    fs.writeFileSync(process.argv[3], String(process.pid));
    const mode = process.argv[2];
    process.stdout.write(JSON.stringify({event:'ready'})+'\\n');
    process.stdin.on('data', (chunk) => {
      if (mode === 'malformed') return process.stdout.write('{broken}\\n');
      if (mode === 'crash') return process.kill(process.pid, 'SIGTERM');
      if (mode === 'truncated') { process.stdout.write('{broken:'); return process.exit(0); }
      if (mode === 'oversize') return process.stdout.write('x'.repeat(16*1024*1024+1));
      if (mode === 'flood') {
        for(let i=0;i<300;i++) process.stdout.write(JSON.stringify({event:'settled',id:i})+'\\n');
        return;
      }
      if (mode === 'queue') return;
      for (const line of String(chunk).trim().split('\\n')) {
        const frame = JSON.parse(line);
        process.stdout.write(JSON.stringify({id:frame.id,kind:'value',value:37})+'\\n');
      }
    });
    process.stdin.on('end', () => {
      fs.writeFileSync(process.argv[4], 'EOF');
      if (mode !== 'ignore') process.exit(0);
    });
    if (mode === 'ignore') process.on('SIGTERM', () => {});
    setInterval(() => {}, 1000);
  `,
    { mode: 0o700 },
  )
  const worker = new Worker(ioPath, {
    workerData: {
      executable,
      endpoint: mode,
      pageId: pidFile,
      script: eofFile,
      outputDir: dir,
    },
  })
  const events = []
  let exited = false
  let workerError
  worker.on('error', (error) => {
    workerError = error
  })
  worker.on('exit', () => {
    exited = true
  })
  worker.on('message', (frame) => {
    events.push(frame)
    if (mode !== 'flood') worker.postMessage({ ack: 1 })
  })
  t.after(async () => {
    if (!exited) {
      worker.postMessage({ stop: true })
      await until(() => exited, 'I/O fixture failed to stop', 7_000).catch(
        async () => worker.terminate(),
      )
    }
    if (fs.existsSync(pidFile)) {
      const pid = Number(fs.readFileSync(pidFile, 'utf8'))
      if (alive(pid)) process.kill(pid, 'SIGKILL')
    }
  })
  await until(
    () => events.some((frame) => frame.event === 'ready') || workerError,
    'native fixture did not become ready',
  )
  if (workerError) throw workerError
  const request = (id = 1) => {
    const shared = new SharedArrayBuffer(8 + 1024)
    worker.postMessage({ frame: { id, op: 'get' }, shared })
    return shared
  }
  const reply = (shared) => {
    const header = new Int32Array(shared, 0, 2)
    assert.notEqual(Atomics.wait(header, 0, 0, 5_000), 'timed-out')
    return JSON.parse(
      new TextDecoder().decode(
        new Uint8Array(shared, 8, Atomics.load(header, 1)),
      ),
    )
  }
  const stopped = async () => {
    await until(
      () => exited || workerError,
      'I/O worker failed to exit after bounded cleanup',
    )
    if (workerError) throw workerError
    const frame = events.find((frame) => frame.event === 'stopped')
    assert(frame, 'stopped evidence must precede worker exit')
    assert.equal(alive(Number(fs.readFileSync(pidFile, 'utf8'))), false)
    return frame
  }
  return { worker, events, request, reply, stopped, eofFile }
}

ioTest(
  'independent I/O wakes a blocked caller and stops only after native EOF and reap',
  async (t) => {
    const fixture = await ioFixture(t, 'normal')
    assert.equal(fixture.reply(fixture.request()).value, 37)
    fixture.worker.postMessage({ stop: true })
    const stopped = await fixture.stopped()
    assert.deepEqual(stopped.exit, { code: 0, signal: null })
    assert.deepEqual(stopped.infrastructureErrors, [])
    assert.equal(fs.readFileSync(fixture.eofFile, 'utf8'), 'EOF')
  },
)

ioTest(
  'I/O parse, crash, and truncated-frame faults release Atomics.wait and reap the native child',
  async (t) => {
    for (const mode of ['malformed', 'crash', 'truncated']) {
      const fixture = await ioFixture(t, mode)
      assert.equal(fixture.reply(fixture.request()).kind, 'transportError')
      const stopped = await fixture.stopped()
      assert(
        stopped.infrastructureErrors.some((error) =>
          /Unreadable native frame|Native worker exited|incomplete frame/.test(
            error,
          ),
        ),
      )
      if (mode === 'malformed')
        assert.equal(fs.readFileSync(fixture.eofFile, 'utf8'), 'EOF')
    }
  },
)

ioTest(
  'I/O close escalates when native ignores EOF and SIGTERM',
  { timeout: 12_000 },
  async (t) => {
    const fixture = await ioFixture(t, 'ignore')
    fixture.worker.postMessage({ stop: true })
    const stopped = await fixture.stopped()
    assert.deepEqual(stopped.exit, { code: null, signal: 'SIGKILL' })
    assert(
      stopped.infrastructureErrors.some((error) =>
        error.includes('EOF cancellation'),
      ),
    )
  },
)

ioTest(
  'I/O frame and request queues remain bounded and release pending callers on failure',
  async (t) => {
    for (const mode of ['oversize', 'queue', 'flood']) {
      const fixture = await ioFixture(t, mode)
      const shared = fixture.request()
      if (mode === 'queue') for (let id = 2; id <= 65; id++) fixture.request(id)
      assert.equal(fixture.reply(shared).kind, 'transportError')
      const stopped = await fixture.stopped()
      assert(
        stopped.infrastructureErrors.some((error) => /exceeded/.test(error)),
      )
    }
  },
)

test('direct parent SIGKILL cancels the case and reaps a detached owned process', {
  timeout: 15_000,
}, async (t) => {
  const dir = temporary(t)
  const owned = path.join(dir, 'owned.cjs')
  const owner = path.join(dir, 'owner.mjs')
  const parent = path.join(dir, 'parent.cjs')
  const parentFile = path.join(dir, 'parent.json')
  const activeFile = path.join(dir, 'active.json')
  fs.writeFileSync(owned, 'setInterval(() => {}, 1000)')
  fs.writeFileSync(
    owner,
    `
    import {spawn} from 'node:child_process';
    import fs from 'node:fs';
    import {runCase} from ${JSON.stringify(modulePath)};
    const configuration = ${JSON.stringify(settings(dir))};
    configuration.parentPid = Number(process.env.NEO_FAULT_PARENT_PID);
    const pair = await runCase(configuration, {
      startBrowser: async () => {
        const child = spawn('node', [${JSON.stringify(owned)}], {detached:true,stdio:'ignore'});
        const exited = new Promise(resolve => child.once('exit', resolve));
        fs.writeFileSync(${JSON.stringify(path.join(dir, 'owned.pid'))}, String(child.pid));
        return {endpoint:'fault-only',pageId:'fault',version:{},snapshot:async()=>({status:'complete'}),
          close:async()=>{ child.kill('SIGTERM'); await exited; }};
      },
      runRunner: async ({kind, signal}) => {
        fs.writeFileSync(${JSON.stringify(activeFile)}, JSON.stringify({owner:process.pid}));
        if (!signal.aborted) await new Promise(resolve => signal.addEventListener('abort',resolve,{once:true}));
        return {kind,exit:{code:null,signal:null},runner:null,journal:null,infrastructureErrors:['cancelled fault fixture']};
      }
    });
    fs.writeFileSync(${JSON.stringify(path.join(dir, 'done'))}, JSON.stringify(pair));
  `,
  )
  fs.writeFileSync(
    parent,
    `
    const {spawn}=require('node:child_process'); const fs=require('node:fs');
    const child=spawn('node',[${JSON.stringify(owner)}],{stdio:'ignore',env:{...process.env,NEO_FAULT_PARENT_PID:String(process.pid)}});
    fs.writeFileSync(${JSON.stringify(parentFile)},JSON.stringify({owner:child.pid}));
    setInterval(()=>{},1000);
  `,
  )
  const child = spawn('node', [parent], { stdio: 'ignore' })
  t.after(() => {
    if (alive(child.pid)) child.kill('SIGKILL')
    for (const file of ['owned.pid', 'parent.json'])
      if (fs.existsSync(path.join(dir, file))) {
        const text = fs.readFileSync(path.join(dir, file), 'utf8')
        const pid = file === 'owned.pid' ? Number(text) : JSON.parse(text).owner
        if (alive(pid)) process.kill(pid, 'SIGKILL')
      }
  })
  await until(
    () => fs.existsSync(activeFile),
    'case did not reach the reference phase',
  )
  const exited = new Promise((resolve) => child.once('exit', resolve))
  child.kill('SIGKILL')
  await exited
  await until(
    () => fs.existsSync(path.join(dir, 'done')),
    'orphan case did not finish bounded cleanup',
  )
  const pair = JSON.parse(fs.readFileSync(path.join(dir, 'pair.json'), 'utf8'))
  assert.match(pair.cancellation, /parent exited or changed identity/)
  assert.equal(pair.phases.neo.runner, null)
  const pid = Number(fs.readFileSync(path.join(dir, 'owned.pid'), 'utf8'))
  assert.equal(alive(pid), false)
})
