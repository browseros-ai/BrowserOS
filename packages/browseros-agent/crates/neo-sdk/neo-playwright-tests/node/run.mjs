#!/usr/bin/env node
/** Neo: Runs the pinned corpus with Playwright's own workers, selection, progress and JSON. */
import { execFileSync, spawn } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { agent, sha256, source } from '../js/prepare.mjs'
import { browserLaunch, browserVersion, cleanupWorkers } from './browser.mjs'
import { curationArgs } from './curation.mjs'
import { runnerOptions } from './options.mjs'
import { materialize } from './overlay.mjs'
import { loadScope, printScores, recordScope, scoreResults } from './scope.mjs'

const argv = process.argv.slice(2)
const options = runnerOptions(argv)
if (options.engine === 'neo') {
  console.error("Neo v2's SDK is not on this branch")
  process.exit(1)
}
const root = path.resolve(
  process.env.NEO_NODE_OUTPUT_DIR ||
    path.resolve(agent, '../../.llm/neo-node-results'),
)
fs.mkdirSync(root, { recursive: true })
const output = fs.realpathSync(
  fs.mkdtempSync(path.join(root, `${options.engine}-`)),
)
console.log(`Results: ${output}`)
// spec.ts (`bun run test:neo-spec`) reads the folder from here instead of from the console, which
// it leaves to Playwright's live terminal reporter.
if (process.env.NEO_NODE_RESULTS_FILE)
  fs.writeFileSync(process.env.NEO_NODE_RESULTS_FILE, output)
const run = {
  mode: options.engine,
  engine: options.engine,
  hostedMode: options.hostedMode,
  browserosVersion: browserVersion(process.env.NEO_E2E_BROWSER),
  browserLaunch: browserLaunch(
    process.env.NEO_E2E_BROWSER,
    undefined,
    options.hostedMode,
  ),
  expectLoop: options.expectLoop && options.engine !== 'claw',
  assertions:
    options.reference || !options.expectLoop
      ? 'playwright'
      : options.engine === 'claw'
        ? 'neo-expect'
        : 'expect-loop',
  argv,
  workers: options.workers,
  startedAt: new Date().toISOString(),
  endedAt: null,
  commit: execFileSync('git', ['rev-parse', 'HEAD'], {
    cwd: agent,
    encoding: 'utf8',
  }).trim(),
  sdkSources:
    options.engine === 'claw'
      ? Object.fromEntries(
          [
            'neo-server/src/runtime/js/bootstrap.js',
            'neo-playwright/assets/prelude.js',
            'neo-server/src/runtime/js/neo_prelude.js',
          ].map((file) => [
            file,
            sha256(fs.readFileSync(new URL(`../../${file}`, import.meta.url))),
          ]),
        )
      : {},
  specsVerified: false,
  contextOptionOmissions: [],
  addonBuild:
    options.engine === 'claw' || !options.debugBuild ? 'release' : 'debug',
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
  run.scope = recordScope(output)
  writeRun()
  if (process.env.PWPAGE_IMPL || process.env.PWDEBUG)
    throw new Error(
      'PWPAGE_IMPL and PWDEBUG override the headless BrowserOS corpus contract',
    )
  if (!process.env.NEO_E2E_BROWSER && !options.args.includes('--list'))
    throw new Error('Set NEO_E2E_BROWSER to the BrowserOS executable')
  const overlay = path.join(output, 'upstream')
  const manifest = materialize(overlay, { ...options, output })
  run.specsVerified = true
  run.curation = manifest.curation
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
    const addonDir = fileURLToPath(
      new URL(
        options.engine === 'claw' ? './claw-addon/' : './addon/',
        import.meta.url,
      ),
    )
    const target = path.resolve(
      process.env.CARGO_TARGET_DIR || path.join(addonDir, 'target'),
    )
    const buildArgs = [
      'build',
      '--manifest-path',
      path.join(addonDir, 'Cargo.toml'),
      '--locked',
      ...(run.addonBuild === 'release' ? ['--release'] : []),
    ]
    run.addon = {
      directory: addonDir,
      target,
      command: ['cargo', ...buildArgs],
    }
    writeRun()
    const build = await command('cargo', buildArgs)
    if (build.code !== 0)
      throw new Error(`Add-on build failed: ${JSON.stringify(build)}`)
    const stem = options.engine === 'claw' ? 'claw_node' : 'neo_node'
    const library =
      process.platform === 'darwin'
        ? `lib${stem}.dylib`
        : process.platform === 'win32'
          ? `${stem}.dll`
          : `lib${stem}.so`
    const temporary = path.join(addonDir, `${stem}.${process.pid}.node`)
    fs.copyFileSync(path.join(target, run.addonBuild, library), temporary)
    fs.renameSync(temporary, path.join(addonDir, `${stem}.node`))
  }
  if (!options.reference && !options.args.includes('--list')) {
    const addonFile =
      process.env.CLAW_NODE_ADDON ||
      fileURLToPath(new URL('./claw-addon/claw_node.node', import.meta.url))
    run.addon = {
      ...run.addon,
      file: addonFile,
      reused: options.skipBuild,
      sha256: sha256(fs.readFileSync(addonFile)),
    }
    writeRun()
  }
  const result = await command(
    process.execPath,
    [
      path.join(source, 'packages/playwright/cli.js'),
      'test',
      '--config',
      manifest.configPath,
      ...(options.curation ? curationArgs(options.args, output) : options.args),
    ],
    {
      cwd: overlay,
      env: {
        ...process.env,
        NEO_NODE_OVERLAY: overlay,
        NEO_NODE_OUTPUT: output,
        NEO_NODE_MODE: run.mode,
        NEO_NODE_HOSTED_MODE: options.hostedMode ? '1' : '0',
        NEO_NODE_EXPECT_LOOP: options.expectLoop ? '1' : '0',
        NEO_NODE_CURATION: options.curation
          ? path.join(output, 'curation.json')
          : '',
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
    if (!options.args.includes('--list')) {
      const scores = scoreResults(
        report,
        loadScope(path.join(output, 'scope.json')),
      )
      fs.writeFileSync(
        path.join(output, 'scores.json'),
        JSON.stringify(scores, null, 2),
      )
      printScores(scores)
    }
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
    run.browserLaunch.workers = fs
      .readdirSync(output)
      .filter((name) => name.startsWith('worker-'))
      .flatMap((name) => {
        const file = path.join(output, name, 'launch.json')
        return fs.existsSync(file)
          ? [{ worker: name, ...JSON.parse(fs.readFileSync(file)) }]
          : []
      })
    run.endedAt = new Date().toISOString()
    run.signal = interrupted
    writeRun()
    process.off('SIGINT', onInt)
    process.off('SIGTERM', onTerm)
  }
}
