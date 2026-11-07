/** Neo: Independent I/O owner. It wakes blocked Atomics.wait callers and reaps its one native child. */
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import { parentPort, workerData } from 'node:worker_threads'

const { executable, endpoint, pageId, script, outputDir } = workerData
const FRAME_LIMIT = 16 * 1024 * 1024
const REQUEST_LIMIT = 64
const EVENT_LIMIT = 256
const child = spawn(executable, [endpoint, pageId, script], {
  stdio: ['pipe', 'pipe', 'pipe'],
})
const stderr = fs.createWriteStream(`${outputDir}/native-stderr.log`, {
  flags: 'wx',
})
const requests = new Map()
const infrastructureErrors = []
const timers = new Set()
let failure
let state = 'running'
let exit = null
let buffered = Buffer.alloc(0)
let pendingEvents = 0
let stderrBytes = 0

function post(frame) {
  if (state === 'stopped') return
  try {
    parentPort.postMessage(frame)
  } catch (error) {
    fail(`I/O parent port failed: ${error}`)
  }
}

function complete(shared, response) {
  const header = new Int32Array(shared, 0, 2)
  const bytes = new TextEncoder().encode(JSON.stringify(response))
  if (bytes.length > shared.byteLength - 8) Atomics.store(header, 1, -1)
  else {
    new Uint8Array(shared, 8, bytes.length).set(bytes)
    Atomics.store(header, 1, bytes.length)
  }
  Atomics.store(header, 0, 1)
  Atomics.notify(header, 0)
}

function failRequests(message) {
  for (const shared of requests.values())
    complete(shared, { kind: 'transportError', message })
  requests.clear()
}

function fail(message) {
  if (!failure) {
    failure = message
    infrastructureErrors.push(message)
    failRequests(message)
    post({ event: 'transportError', message })
  }
  stop()
}

function after(ms, operation) {
  const timer = setTimeout(() => {
    timers.delete(timer)
    operation()
  }, ms)
  timers.add(timer)
}

function kill(signal) {
  if (exit || !child.pid) return
  try {
    child.kill(signal)
  } catch (error) {
    if (error.code !== 'ESRCH')
      infrastructureErrors.push(`Native ${signal}: ${error}`)
  }
}

function finalize() {
  if (state === 'stopped') return
  for (const timer of timers) clearTimeout(timer)
  timers.clear()
  failRequests(failure || 'Native IPC stopped')
  post({
    event: 'stopped',
    exit: exit || { code: null, signal: null },
    infrastructureErrors,
  })
  state = 'stopped'
  // Closing the port only after child close preserves EOF cancellation and reaping.
  // Worker.terminate() would tear down this thread before it could stop the native run.
  parentPort.close()
  process.off('uncaughtException', onUncaught)
  process.off('unhandledRejection', onRejection)
}

function stop() {
  if (state !== 'running') return
  state = 'stopping'
  failRequests(failure || 'Native IPC stopped')
  child.stdin.end()
  // EOF cancels the Rust runtime first. Signals are a finite fallback for a child
  // stuck before or during runtime setup, including a failed transport handshake.
  after(2_000, () => {
    if (exit) return
    infrastructureErrors.push(
      'Native worker did not exit after EOF cancellation',
    )
    kill('SIGTERM')
    after(1_000, () => {
      if (exit) return
      kill('SIGKILL')
      after(2_000, () => {
        if (exit) return
        infrastructureErrors.push('Native worker did not reap after SIGKILL')
        child.unref()
        child.stdin.destroy()
        child.stdout.destroy()
        child.stderr.destroy()
        stderr.end()
        finalize()
      })
    })
  })
}

function frame(text) {
  try {
    const value = JSON.parse(text)
    if (!value || typeof value !== 'object')
      throw new Error('Expected frame object')
    if (value.event) {
      if (++pendingEvents > EVENT_LIMIT)
        return fail('Native event queue exceeded 256 frames')
      post(value)
    } else {
      const shared = requests.get(value.id)
      if (!shared) return fail(`Uncorrelated native reply ${value.id}`)
      requests.delete(value.id)
      complete(shared, value)
    }
  } catch (error) {
    fail(`Unreadable native frame: ${error}`)
  }
}

// readline would retain an unbounded unterminated line. Bound each byte frame before
// parsing, while still allowing one stream chunk to contain several valid replies.
child.stdout.on('data', (chunk) => {
  if (failure) return
  let start = 0
  for (;;) {
    const end = chunk.indexOf(10, start)
    const part = chunk.subarray(start, end === -1 ? chunk.length : end)
    if (buffered.length + part.length > FRAME_LIMIT)
      return fail('Native frame exceeded 16 MiB')
    buffered = buffered.length ? Buffer.concat([buffered, part]) : part
    if (end === -1) return
    frame(buffered.toString('utf8'))
    buffered = Buffer.alloc(0)
    if (failure) return
    start = end + 1
    if (start === chunk.length) return
  }
})
child.stdout.on('end', () => {
  if (buffered.length) fail('Native stdout ended with an incomplete frame')
})
child.stderr.on('data', (chunk) => {
  stderrBytes += chunk.length
  if (stderrBytes > FRAME_LIMIT) return fail('Native stderr exceeded 16 MiB')
  if (!stderr.write(chunk)) child.stderr.pause()
})
stderr.on('drain', () => child.stderr.resume())
child.stderr.on('end', () => stderr.end())
for (const stream of [child.stdin, child.stdout, child.stderr, stderr])
  stream.on('error', (error) => fail(`Native stream failed: ${error}`))
child.on('error', (error) => fail(`Native spawn failed: ${error}`))
child.on('exit', (code, signal) => {
  exit = { code, signal }
  post({ event: 'exit', code, signal })
  if (code !== 0 || signal !== null)
    fail(`Native worker exited (${code}, ${signal})`)
  // exit reaps the child, but inherited or damaged pipes can delay close forever.
  // Those diagnostics are incomplete and cannot hold the I/O thread past its guard.
  after(2_000, () => {
    if (state === 'stopped') return
    infrastructureErrors.push(
      'Native worker streams did not close after process exit',
    )
    child.stdin.destroy()
    child.stdout.destroy()
    child.stderr.destroy()
    stderr.destroy()
    finalize()
  })
})
child.on('close', () => {
  // close follows exit and stream closure. No child or pipe is still owned when the
  // host sees stopped, so it can publish the journal without racing native writers.
  if (!exit)
    infrastructureErrors.push('Native worker closed without exit evidence')
  if (stderr.writableFinished) finalize()
  else stderr.end(finalize)
})
parentPort.on('message', ({ frame, shared, stop: requestedStop, ack }) => {
  if (ack) {
    pendingEvents = Math.max(0, pendingEvents - ack)
    return
  }
  if (requestedStop) return stop()
  if (failure || state !== 'running' || exit)
    return complete(shared, {
      kind: 'transportError',
      message:
        failure || (exit ? 'Native worker exited' : 'Native IPC stopped'),
    })
  if (
    !frame ||
    !Number.isSafeInteger(frame.id) ||
    frame.id < 1 ||
    requests.has(frame.id)
  ) {
    complete(shared, {
      kind: 'transportError',
      message: 'Invalid or duplicate native request',
    })
    return fail('Invalid or duplicate native request')
  }
  const text = `${JSON.stringify(frame)}\n`
  if (
    Buffer.byteLength(text) > FRAME_LIMIT ||
    requests.size >= REQUEST_LIMIT ||
    child.stdin.writableLength + Buffer.byteLength(text) > FRAME_LIMIT
  ) {
    complete(shared, {
      kind: 'transportError',
      message: 'Native request frame or queue exceeded its budget',
    })
    return fail('Native request frame or queue exceeded its budget')
  }
  requests.set(frame.id, shared)
  child.stdin.write(text, (error) => {
    if (error) fail(`Native stdin failed: ${error}`)
  })
})
parentPort.on('close', stop)
const onUncaught = (error) => fail(`I/O worker failed: ${error}`)
const onRejection = (error) => fail(`I/O worker rejected: ${error}`)
process.on('uncaughtException', onUncaught)
process.on('unhandledRejection', onRejection)
