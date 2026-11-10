#!/usr/bin/env node
/** Neo: Runs the pinned corpus with Playwright's own workers, selection, progress and JSON. */
import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { agent, sha256, source } from '../js/prepare.mjs'
import { cleanupWorkers } from './browser.mjs'
import { runnerOptions } from './options.mjs'
import { materialize } from './overlay.mjs'

const argv = process.argv.slice(2)
const options = runnerOptions(argv)
const root = path.resolve(
  process.env.NEO_NODE_OUTPUT_DIR ||
    path.resolve(agent, '../../.llm/neo-node-results'),
)
fs.mkdirSync(root, { recursive: true })
const output = fs.realpathSync(
  fs.mkdtempSync(path.join(root, options.reference ? 'reference-' : 'neo-')),
)
console.log(`Results: ${output}`)
const run = {
  mode: options.reference ? 'reference' : 'neo',
  argv,
  workers: options.workers,
  runnerExpect: !options.reference && options.runnerExpect,
  startedAt: new Date().toISOString(),
  endedAt: null,
  commit: execFileSync('git', ['rev-parse', 'HEAD'], {
    cwd: agent,
    encoding: 'utf8',
  }).trim(),
  sdkSha256: sha256(
    fs.readFileSync(
      new URL('../../neo-quickjs-injected/dist/sdk.js', import.meta.url),
    ),
  ),
  specsVerified: false,
  contextOptionOmissions: [],
  addonBuild: options.debugBuild ? 'debug' : 'release',
}
const writeRun = () =>
  fs.writeFileSync(path.join(output, 'run.json'), JSON.stringify(run, null, 2))
writeRun()
let active
let interrupted
let escalation
const interrupt = (signal) => {
  interrupted ||= signal
  if (active?.pid && active.exitCode === null && active.signalCode === null) {
    try {
      process.kill(-active.pid, signal)
    } catch (error) {
      if (error.code !== 'ESRCH') throw error
    }
    escalation ??= setTimeout(() => {
      if (
        active?.pid &&
        active.exitCode === null &&
        active.signalCode === null
      ) {
        try {
          process.kill(-active.pid, 'SIGKILL')
        } catch (error) {
          if (error.code !== 'ESRCH') throw error
        }
      }
    }, 10_000)
  }
}
const onInt = () => interrupt('SIGINT')
const onTerm = () => interrupt('SIGTERM')
process.on('SIGINT', onInt)
process.on('SIGTERM', onTerm)
async function command(executable, args, settings = {}) {
  if (interrupted) throw new Error(`Launcher interrupted: ${interrupted}`)
  active = spawn(executable, args, {
    stdio: 'inherit',
    detached: true,
    ...settings,
  })
  try {
    return await new Promise((resolve, reject) => {
      active.once('error', reject)
      active.once('exit', (code, signal) => resolve({ code, signal }))
    })
  } finally {
    active = undefined
    clearTimeout(escalation)
    escalation = undefined
  }
}
try {
  if (process.env.PWPAGE_IMPL || process.env.PWDEBUG)
    throw new Error(
      'PWPAGE_IMPL and PWDEBUG override the headless BrowserOS corpus contract',
    )
  if (!process.env.NEO_E2E_BROWSER && !options.args.includes('--list'))
    throw new Error('Set NEO_E2E_BROWSER to the BrowserOS executable')
  const overlay = path.join(output, 'upstream')
  const manifest = materialize(overlay, { ...options, output })
  run.specsVerified = true
  run.contextOptionOmissions = manifest.contextOptionOmissions
  run.headlessWaivers = manifest.edits.filter((edit) =>
    edit.reason.startsWith('owner-approved'),
  )
  writeRun()
  if (
    !options.reference &&
    !options.skipBuild &&
    !options.args.includes('--list')
  ) {
    const addonDir = fileURLToPath(new URL('./addon/', import.meta.url))
    const target = path.resolve(
      process.env.CARGO_TARGET_DIR || path.join(addonDir, 'target'),
    )
    const build = await command('cargo', [
      'build',
      '--manifest-path',
      path.join(addonDir, 'Cargo.toml'),
      '--locked',
      ...(options.debugBuild ? [] : ['--release']),
    ])
    if (build.code !== 0)
      throw new Error(`Add-on build failed: ${JSON.stringify(build)}`)
    const library =
      process.platform === 'darwin'
        ? 'libneo_node.dylib'
        : process.platform === 'win32'
          ? 'neo_node.dll'
          : 'libneo_node.so'
    const temporary = path.join(addonDir, `neo_node.${process.pid}.node`)
    fs.copyFileSync(path.join(target, run.addonBuild, library), temporary)
    fs.renameSync(temporary, path.join(addonDir, 'neo_node.node'))
  }
  const result = await command(
    process.execPath,
    [
      path.join(source, 'packages/playwright/cli.js'),
      'test',
      '--config',
      manifest.configPath,
      ...options.args,
    ],
    {
      cwd: overlay,
      env: {
        ...process.env,
        NEO_NODE_OVERLAY: overlay,
        NEO_NODE_OUTPUT: output,
        NEO_NODE_MODE: run.mode,
        NEO_NODE_RUNNER_EXPECT: String(run.runnerExpect),
        NEO_NODE_LAUNCHER_PID: String(process.pid),
        PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: '1',
        PWTEST_UNDER_TEST: '1',
      },
    },
  )
  fs.writeFileSync(path.join(output, 'exit.json'), JSON.stringify(result))
  if (fs.existsSync(path.join(output, 'results.json'))) {
    const report = JSON.parse(
      fs.readFileSync(path.join(output, 'results.json'), 'utf8'),
    )
    console.log(`Playwright totals: ${JSON.stringify(report.stats)}`)
  }
  process.exitCode = interrupted
    ? interrupted === 'SIGINT'
      ? 130
      : 143
    : (result.code ?? 1)
} catch (error) {
  run.error = error.stack || error.message
  console.error(error)
  process.exitCode = interrupted ? (interrupted === 'SIGINT' ? 130 : 143) : 1
} finally {
  try {
    await cleanupWorkers(output)
  } catch (error) {
    run.cleanupError = error.stack || error.message
    console.error(error)
    process.exitCode = 1
  } finally {
    run.endedAt = new Date().toISOString()
    run.signal = interrupted
    writeRun()
    process.off('SIGINT', onInt)
    process.off('SIGTERM', onTerm)
  }
}
