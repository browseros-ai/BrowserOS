#!/usr/bin/env node
/** A process fixture implementing Chromium's port-file/readiness contract for launcher checks. */
/* biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Process fixture controls are not cached Turbo task inputs. */
import { spawn } from 'node:child_process'
import fs from 'node:fs'
import http from 'node:http'
import net from 'node:net'
import path from 'node:path'

const args = process.argv.slice(2)
if (args[0] === 'command') {
  let child
  if (process.env.FIXTURE_COMMAND_CHILD === '1') {
    child = spawn(process.execPath, [import.meta.filename, 'linger'], {
      stdio: 'ignore',
    })
    child.unref()
  }
  process.stdout.write(
    `${JSON.stringify({
      args: args.slice(1),
      cwd: process.cwd(),
      pid: process.pid,
      home: process.env.HOME,
      port: Number(process.env.BROWSEROS_CDP_PORT),
      child: child?.pid,
    })}\n`,
  )
  process.exitCode = Number(process.env.FIXTURE_COMMAND_EXIT || '0')
  if (process.env.FIXTURE_COMMAND_WAIT === '1') setInterval(() => {}, 1000)
} else if (args[0] === 'linger') {
  process.on('SIGTERM', () => {})
  setInterval(() => {}, 1000)
} else {
  const profile = args
    .find((arg) => arg.startsWith('--user-data-dir='))
    ?.slice('--user-data-dir='.length)
  if (!profile) throw new Error('Missing browser profile')
  if (process.env.FIXTURE_BROWSER_MODE === 'exit') process.exit(7)
  if (process.env.FIXTURE_BROWSER_MODE === 'unowned')
    process.title = 'test-browser-fixture-unowned'
  if (process.env.FIXTURE_BROWSER_SOCKET === '1') {
    // Chromium binds its singleton socket beneath TMPDIR; long artifact paths cannot host it.
    const directory = path.join(
      process.env.TMPDIR,
      'org.chromium.Chromium.fixture',
    )
    fs.mkdirSync(directory)
    const socket = net.createServer()
    await new Promise((resolve, reject) => {
      socket.once('error', reject)
      socket.listen(path.join(directory, 'SingletonSocket'), resolve)
    })
    if (!fs.existsSync(path.join(directory, 'SingletonSocket')))
      throw new Error('Singleton socket path was truncated')
  }
  const server = http.createServer((_request, response) => {
    response.setHeader('content-type', 'application/json')
    response.end(
      JSON.stringify({
        Browser: 'Fixture/1',
        webSocketDebuggerUrl: `ws://127.0.0.1:${server.address().port}/devtools/browser/fixture`,
      }),
    )
  })
  server.listen(0, '127.0.0.1', () => {
    const port = server.address().port
    fs.writeFileSync(
      process.env.FIXTURE_BROWSER_RECORD,
      JSON.stringify({
        pid: process.pid,
        profile,
        home: process.env.HOME,
        temp: process.env.TMPDIR,
        args,
        port,
      }),
    )
    const publish = () =>
      fs.writeFileSync(
        path.join(profile, 'DevToolsActivePort'),
        `${port}\n/devtools/browser/fixture\n`,
      )
    const barrier = process.env.FIXTURE_BROWSER_READY_FILE
    if (barrier) {
      const timer = setInterval(() => {
        if (fs.existsSync(barrier)) {
          clearInterval(timer)
          publish()
        }
      }, 20)
    } else publish()
  })
}
