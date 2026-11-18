// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: This standalone Node harness is not a Turbo task.
/** Real BrowserOS, exact v1 SDK, and a child Node process that must exit without process.exit. */
import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { once } from 'node:events'
import fs from 'node:fs'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { setTimeout as sleep } from 'node:timers/promises'
import { fileURLToPath } from 'node:url'

import { isAlive, stopBrowser } from './browser.mjs'
import { openClaw } from './claw-loader.mjs'

const filename = fileURLToPath(import.meta.url)

async function startBrowser(executable, directory, hosted) {
  const profile = fs.mkdtempSync(path.join(directory, 'profile-'))
  const env = { ...process.env }
  // The primary run matches v2 run B. The second run measures BrowserOS's hosted-mode patch.
  if (hosted) env.BROWSERCLAW_HOSTED_MODE = '1'
  else delete env.BROWSERCLAW_HOSTED_MODE
  const log = fs.openSync(path.join(directory, 'browser.log'), 'wx')
  let child
  try {
    child = spawn(
      executable,
      [
        '--headless=new',
        '--disable-browseros-server',
        '--disable-browseros-server-updater',
        '--disable-browseros-extensions',
        '--remote-debugging-port=0',
        `--user-data-dir=${profile}`,
        '--no-first-run',
        '--lang=en-US',
        '--window-size=1280,720',
        '--force-device-scale-factor=1',
        'about:blank',
      ],
      { detached: true, env, stdio: ['ignore', log, log] },
    )
  } finally {
    fs.closeSync(log)
  }
  let exit
  const exited = new Promise((resolve) => {
    child.once('exit', (code, signal) => {
      exit = { code, signal }
      resolve()
    })
    child.once('error', (error) => {
      exit = { error: error.message }
      resolve()
    })
  })
  const owner = { pid: child.pid, profile, hosted }
  fs.writeFileSync(path.join(directory, 'owner.json'), JSON.stringify(owner))
  let closing
  const close = () =>
    (closing ??= (async () => {
      await stopBrowser(owner)
      await exited
      fs.rmSync(profile, { recursive: true, force: true })
      fs.writeFileSync(
        path.join(directory, 'cleanup.json'),
        JSON.stringify({
          ...owner,
          exit,
          groupGone: !child.pid || !isAlive(-child.pid),
        }),
      )
    })())
  try {
    const deadline = Date.now() + 30_000
    while (Date.now() < deadline) {
      if (exit)
        throw new Error(`BrowserOS exited at startup: ${JSON.stringify(exit)}`)
      try {
        const port = Number(
          fs
            .readFileSync(path.join(profile, 'DevToolsActivePort'), 'utf8')
            .split('\n')[0],
        )
        if (Number.isInteger(port) && port > 0 && port !== 9200)
          return { port, profile, close }
      } catch (error) {
        if (error.code !== 'ENOENT') throw error
      }
      await sleep(50)
    }
    throw new Error('BrowserOS did not publish DevToolsActivePort')
  } catch (error) {
    await close()
    throw error
  }
}

async function within(promise, milliseconds = 5000) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`smoke wait exceeded ${milliseconds}ms`)),
          milliseconds,
        )
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function childSmoke() {
  const hosted = process.env.CLAW_SMOKE_HOSTED === '1'
  const sentinel = new Error('listener failure reaches onUncaught')
  let receivedError
  const uncaught = new Promise((resolve) => {
    receivedError = resolve
  })
  const sdk = await openClaw(
    Number(process.env.CLAW_SMOKE_PORT),
    {
      stateDir: process.env.CLAW_SMOKE_STATE,
      executionDir: process.env.CLAW_SMOKE_EXECUTION,
      browserosVersion: process.env.CLAW_SMOKE_VERSION,
      hostedMode: hosted,
    },
    { onUncaught: (error) => receivedError(error) },
  )
  try {
    assert.equal(Object.hasOwn(globalThis, '__neo'), false)
    assert.equal(sdk.params.page, sdk.page)
    assert.equal(sdk.params.context, sdk.context)
    assert.equal(sdk.params.browser, sdk.browser)
    assert(sdk.Buffer.from('claw') instanceof Uint8Array)
    assert(new sdk.params.NeoError({}) instanceof Error)
    assert.equal(Object.keys(sdk.sources).length, 3)
    const init = JSON.parse(sdk.native.init)
    const creating = sdk.context.newPage()
    assert(creating instanceof Promise, 'SDK promises belong to the main realm')
    const page = await creating
    let consoleReceived
    const consoleMessage = new Promise((resolve) => {
      consoleReceived = resolve
    })
    page.on('console', (message) => {
      if (message.text() === 'claw smoke console') consoleReceived(message)
      if (message.text() === 'throw from listener') throw sentinel
    })
    await page.goto(process.env.CLAW_SMOKE_URL)
    assert.equal(await page.title(), 'Claw smoke')
    assert.equal(await page.locator('h1').textContent(), 'Real SDK')
    assert.equal(await page.evaluate(() => 1 + 1), 2)
    const viewport = await page.evaluate(() => ({
      width: innerWidth,
      height: innerHeight,
    }))
    const consoleFired = await within(consoleMessage, 2000).then(
      () => true,
      () => false,
    )
    if (!hosted)
      assert(consoleFired, 'page.on(console) must fire in the primary smoke')
    let uncaughtRouted = false
    if (consoleFired) {
      await page.evaluate(() => console.log('throw from listener'))
      assert.equal(await within(uncaught), sentinel)
      uncaughtRouted = true
    }
    const refused = (error) =>
      error instanceof Error &&
      error instanceof sdk.params.NeoError &&
      error.code === 'method_unavailable'
    await assert.rejects(async () => page.route('**/*', () => {}), refused)
    await assert.rejects(async () => sdk.browser.newContext(), refused)
    const setContent = await (async () => {
      await page.setContent(
        '<title>Written content</title><h1>Written heading</h1>',
      )
      assert.equal(await page.title(), 'Written content')
      assert.equal(await page.locator('h1').textContent(), 'Written heading')
      return { ok: true }
    })().catch((error) => ({ ok: false, error: error.message }))
    const popupResult = page.waitForEvent('popup', { timeout: 2000 }).then(
      async (popup) => {
        await popup.close()
        return { ok: true }
      },
      (error) => ({ ok: false, error: error.message }),
    )
    await page.evaluate(() => {
      window.open('about:blank')
    })
    const popup = await popupResult
    await page.close()
    const firstFinish = sdk.finish()
    assert.equal(
      firstFinish,
      sdk.finish(),
      'the loader shares its finish promise',
    )
    const finished = JSON.parse(await within(firstFinish))
    assert.equal(finished.status, 'ok')
    const proof = {
      hosted,
      browserosVersion: process.env.CLAW_SMOKE_VERSION,
      init,
      viewport,
      sources: sdk.sources,
      consoleFired,
      uncaughtRouted,
      setContent,
      popup,
      finish: finished.status,
    }
    fs.writeFileSync(
      process.env.CLAW_SMOKE_PROOF,
      JSON.stringify(proof, null, 2),
    )
    console.log(JSON.stringify(proof))
  } finally {
    await sdk.finish()
  }
}

if (process.argv.includes('--claw-smoke-child')) {
  await childSmoke()
} else {
  test('Claw SDK in Node drives real BrowserOS and releases its resources', {
    skip: !process.env.NEO_E2E_BROWSER,
    timeout: 120_000,
  }, async (t) => {
    const output = process.env.NEO_NODE_OUTPUT_DIR || os.tmpdir()
    fs.mkdirSync(output, { recursive: true })
    const directory = fs.mkdtempSync(path.join(output, 'claw-smoke-'))
    const executable = process.env.NEO_E2E_BROWSER
    const plist = path.join(
      path.dirname(path.dirname(executable)),
      'Info.plist',
    )
    const version =
      process.platform === 'darwin'
        ? execFileSync(
            '/usr/libexec/PlistBuddy',
            ['-c', 'Print :CFBundleShortVersionString', plist],
            { encoding: 'utf8' },
          ).trim()
        : process.env.NEO_E2E_BROWSEROS_VERSION
    const server = http.createServer((_request, response) => {
      response.setHeader('content-type', 'text/html')
      response.end(
        '<title>Claw smoke</title><h1>Real SDK</h1><script>console.log("claw smoke console")</script>',
      )
    })
    server.listen(0, '127.0.0.1')
    await once(server, 'listening')
    t.after(async () => {
      server.closeAllConnections()
      await new Promise((resolve) => server.close(resolve))
    })
    const browser = await startBrowser(
      executable,
      directory,
      process.env.CLAW_SMOKE_HOSTED === '1',
    )
    t.after(() => browser.close())
    const child = spawn(process.execPath, [filename, '--claw-smoke-child'], {
      env: {
        ...process.env,
        CLAW_SMOKE_PORT: String(browser.port),
        CLAW_SMOKE_STATE: path.join(directory, 'state'),
        CLAW_SMOKE_EXECUTION: path.join(browser.profile, '.browseros'),
        CLAW_SMOKE_VERSION: version || '',
        CLAW_SMOKE_URL: `http://127.0.0.1:${server.address().port}/`,
        CLAW_SMOKE_PROOF: path.join(directory, 'proof.json'),
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    let outputText = ''
    child.stdout.on('data', (chunk) => {
      outputText += chunk
    })
    child.stderr.on('data', (chunk) => {
      outputText += chunk
    })
    const exited = once(child, 'exit')
    // Only our child is signalled on a hang; normal success must exit on its own.
    t.after(() => {
      if (child.exitCode === null && child.signalCode === null)
        child.kill('SIGKILL')
    })
    const [code, signal] = await within(exited, 90_000)
    fs.writeFileSync(path.join(directory, 'node.log'), outputText)
    assert.equal(signal, null, outputText)
    assert.equal(code, 0, outputText)
    await browser.close()
    t.diagnostic(`${directory}\n${outputText.trim()}`)
    if (!process.env.NEO_NODE_OUTPUT_DIR)
      fs.rmSync(directory, { recursive: true, force: true })
  })
}
