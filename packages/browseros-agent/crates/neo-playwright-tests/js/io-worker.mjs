/** Independent I/O thread. The test thread may block while native getters await live CDP. */

import { spawn } from 'node:child_process'
import fs from 'node:fs'
import readline from 'node:readline'
import { parentPort, workerData } from 'node:worker_threads'

const { executable, endpoint, pageId, script, outputDir } = workerData
const child = spawn(executable, [endpoint, pageId, script], {
  stdio: ['pipe', 'pipe', 'pipe'],
})
const stderr = fs.createWriteStream(`${outputDir}/native-stderr.log`, {
  flags: 'wx',
})
child.stderr.pipe(stderr)
const requests = new Map()
let dead
function complete(shared, response) {
  const header = new Int32Array(shared, 0, 2)
  const bytes = new TextEncoder().encode(JSON.stringify(response))
  if (bytes.length > shared.byteLength - 8) {
    Atomics.store(header, 1, -1)
  } else {
    new Uint8Array(shared, 8, bytes.length).set(bytes)
    Atomics.store(header, 1, bytes.length)
  }
  Atomics.store(header, 0, 1)
  Atomics.notify(header, 0)
}
function fail(message) {
  if (dead) return
  dead = message
  for (const shared of requests.values())
    complete(shared, { kind: 'transportError', message })
  requests.clear()
  parentPort.postMessage({ event: 'transportError', message })
}
readline.createInterface({ input: child.stdout }).on('line', (text) => {
  try {
    const frame = JSON.parse(text)
    if (frame.event) parentPort.postMessage(frame)
    else {
      const shared = requests.get(frame.id)
      if (!shared) return fail(`Uncorrelated native reply ${frame.id}`)
      requests.delete(frame.id)
      complete(shared, frame)
    }
  } catch (error) {
    fail(`Unreadable native frame: ${error}`)
  }
})
child.on('error', (error) => fail(String(error)))
child.on('exit', (code, signal) => {
  parentPort.postMessage({ event: 'exit', code, signal })
  fail(`Native worker exited (${code}, ${signal})`)
})
parentPort.on('message', ({ frame, shared, stop }) => {
  if (stop) {
    child.stdin.end()
    return
  }
  if (dead) return complete(shared, { kind: 'transportError', message: dead })
  requests.set(frame.id, shared)
  child.stdin.write(`${JSON.stringify(frame)}\n`, (error) => {
    if (error) fail(String(error))
  })
})
