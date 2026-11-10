#!/usr/bin/env node
/** Neo: One command builds the add-on and runs the untouched upstream file under its own runner. */
import { spawn, spawnSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import { agent, source } from '../js/prepare.mjs'
import { leasePorts } from '../js/run-case.mjs'
import { cleanupWorkers } from './browser.mjs'
import { materialize } from './overlay.mjs'

const args = process.argv.slice(2)
const reference = args.includes('--reference')
const skipBuild = args.includes('--skip-build')
const passedArgs = args.filter(
  (arg) => !['--reference', '--skip-build'].includes(arg),
)
const root =
  process.env.NEO_NODE_OUTPUT_DIR ||
  path.resolve(agent, '../../.llm/neo-node-spike')
fs.mkdirSync(root, { recursive: true })
const output = fs.mkdtempSync(
  path.join(root, reference ? 'playwright-' : 'neo-'),
)
console.log(`Results: ${output}`)
if (!process.env.NEO_E2E_BROWSER)
  throw new Error('Set NEO_E2E_BROWSER to the BrowserOS executable')
if (!reference && !skipBuild) {
  const addonDir = fileURLToPath(new URL('./addon/', import.meta.url))
  const target = process.env.CARGO_TARGET_DIR || path.join(addonDir, 'target')
  const build = spawnSync(
    'cargo',
    ['build', '--manifest-path', path.join(addonDir, 'Cargo.toml'), '--locked'],
    { stdio: 'inherit' },
  )
  if (build.error) throw build.error
  if (build.status !== 0) process.exit(build.status || 1)
  const library =
    process.platform === 'darwin'
      ? 'libneo_node.dylib'
      : process.platform === 'win32'
        ? 'neo_node.dll'
        : 'libneo_node.so'
  fs.copyFileSync(
    path.join(target, 'debug', library),
    path.join(addonDir, 'neo_node.node'),
  )
}
const overlay = path.join(output, 'upstream')
const config = materialize(overlay, { reference, output })
const lease = await leasePorts()
try {
  const child = spawn(
    process.execPath,
    [
      path.join(source, 'packages/playwright/cli.js'),
      'test',
      '--config',
      config,
      ...passedArgs,
    ],
    {
      cwd: overlay,
      stdio: 'inherit',
      env: {
        ...process.env,
        NEO_NODE_OVERLAY: overlay,
        NEO_NODE_OUTPUT: output,
        NEO_TESTS_PORT: String(lease.port),
        PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: '1',
      },
    },
  )
  const result = await new Promise((resolve, reject) => {
    child.once('error', reject)
    child.once('exit', (code, signal) => resolve({ code, signal }))
  })
  fs.writeFileSync(path.join(output, 'exit.json'), JSON.stringify(result))
  if (fs.existsSync(path.join(output, 'results.json'))) {
    const report = JSON.parse(
      fs.readFileSync(path.join(output, 'results.json'), 'utf8'),
    )
    console.log(`Playwright totals: ${JSON.stringify(report.stats)}`)
  }
  process.exitCode = result.code ?? 1
} finally {
  try {
    await cleanupWorkers(output)
  } finally {
    lease.release()
  }
}
