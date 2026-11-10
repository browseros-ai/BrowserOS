/** Neo: Keeps browser/profile/port ownership alive across worker failure and launcher signals. */
import fs from 'node:fs'
import path from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'
import { leasePorts } from '../js/run-case.mjs'
import { cleanupWorker, isAlive, startOwnedBrowser } from './browser.mjs'

const output = process.argv[2]
const controller = new AbortController()
const stop = () => controller.abort(new Error('Browser owner cancelled'))
process.on('disconnect', stop)
process.on('SIGINT', stop)
process.on('SIGTERM', stop)
const send = (message) => {
  if (process.connected) process.send(message, () => {})
}
let started = false
process.on('message', (message) => {
  if (message.type === 'stop') stop()
  if (message.type === 'start' && !started) {
    started = true
    run(message).catch((error) => {
      console.error(error)
      process.exitCode = 1
    })
  }
})

async function run({ executable, slot, launcherPid }) {
  let owned
  let lease
  let slotPath
  let watch
  try {
    fs.writeFileSync(
      path.join(output, 'guardian.json'),
      JSON.stringify({ pid: process.pid, launcherPid, slot }),
    )
    if (launcherPid) {
      const check = () => {
        if (!isAlive(launcherPid)) stop()
      }
      check()
      watch = setInterval(check, 250)
    }
    // Playwright may replace a dead worker before its guardian finishes. Hold its parallel slot
    // through cleanup so the replacement cannot become a fifth browser.
    if (slot !== undefined) {
      const target = path.join(path.dirname(output), `.browser-slot-${slot}`)
      while (!slotPath) {
        controller.signal.throwIfAborted()
        try {
          fs.mkdirSync(target)
          slotPath = target
          fs.writeFileSync(
            path.join(target, 'owner.json'),
            JSON.stringify({ pid: process.pid, output }),
          )
        } catch (error) {
          if (error.code !== 'EEXIST') throw error
          const record = path.join(target, 'owner.json')
          if (fs.existsSync(record)) {
            const previous = JSON.parse(fs.readFileSync(record))
            if (!isAlive(previous.pid)) {
              if (path.dirname(previous.output) !== path.dirname(output))
                throw new Error('Invalid previous worker slot')
              await cleanupWorker(previous.output)
              fs.rmSync(target, { recursive: true })
              continue
            }
          }
          await sleep(100, undefined, { signal: controller.signal })
        }
      }
    }
    lease = await leasePorts({ signal: controller.signal })
    fs.writeFileSync(
      path.join(output, 'ports.json'),
      JSON.stringify({ port: lease.port }),
    )
    owned = await startOwnedBrowser(executable, output, controller.signal)
    send({
      type: 'ready',
      port: owned.port,
      pid: owned.pid,
      profile: owned.profile,
      serverPort: lease.port,
    })
    await Promise.race([
      owned.exited.then((exit) => {
        send({ type: 'browser-exited', exit })
      }),
      new Promise((resolve) => {
        controller.signal.addEventListener('abort', resolve, { once: true })
        if (controller.signal.aborted) resolve()
      }),
    ])
  } catch (error) {
    if (!controller.signal.aborted)
      send({ type: 'error', message: error.stack || error.message })
  } finally {
    try {
      await owned?.close()
      if (slotPath) fs.rmSync(slotPath, { recursive: true })
    } catch (error) {
      send({ type: 'error', message: error.stack || error.message })
      console.error(error)
      process.exitCode = 1
    } finally {
      lease?.release()
      if (lease)
        fs.writeFileSync(
          path.join(output, 'ports.json'),
          JSON.stringify({ port: lease.port, released: true }),
        )
      clearInterval(watch)
      if (process.connected) process.disconnect()
    }
  }
}
