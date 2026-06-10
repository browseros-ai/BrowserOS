/**
 * End-to-end test for CAPTCHA solver integration.
 *
 * Runs a single eval task against Google's reCAPTCHA demo page:
 *   1. Launches BrowserOS (headed) with NopeCHA extension loaded
 *   2. Agent navigates to reCAPTCHA demo, fills form
 *   3. CaptchaWaiter polls until NopeCHA solves the CAPTCHA
 *   4. Screenshot is captured AFTER solve
 *   5. Verifies: task completed, screenshots exist, metadata saved
 *
 * Prerequisites:
 *   - NOPECHA_API_KEY env var set
 *   - FIREWORKS_API_KEY env var set (or swap agent config)
 *   - NopeCHA extension at extensions/nopecha/ (run the install step from CI)
 *   - BrowserOS binary available
 *
 * Run:
 *   bun --env-file=apps/eval/.env.development apps/eval/tests/e2e/captcha-e2e.ts
 */

import { existsSync, readdirSync, readFileSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { BrowserOSAppManager } from '../../src/runner/browseros-app-manager'
import { createTaskRunPipeline as createTaskExecutor } from '../../src/runs/task-run-pipeline'
import { EvalConfigSchema } from '../../src/types/config'
import { TaskSchema } from '../../src/types/task'

const HERE = dirname(fileURLToPath(import.meta.url))
const OUTPUT_DIR = join(HERE, 'results')

const EVAL_CONFIG = {
  agent: {
    type: 'single' as const,
    provider: 'openai-compatible' as const,
    model: 'accounts/fireworks/models/kimi-k2p5',
    apiKey: 'FIREWORKS_API_KEY',
    baseUrl: 'https://api.fireworks.ai/inference/v1',
    supportsImages: true,
  },
  dataset: 'inline',
  num_workers: 1,
  restart_server_per_task: true,
  browseros: {
    server_url: 'http://127.0.0.1:9110',
    base_cdp_port: 9010,
    base_server_port: 9110,
    base_extension_port: 9310,
    load_extensions: false,
    headless: false,
  },
  captcha: { api_key_env: 'NOPECHA_API_KEY' },
  timeout_ms: 120000,
}

const TASK = {
  query_id: 'captcha-e2e-1',
  dataset: 'captcha-test',
  query:
    "Go to the Google reCAPTCHA demo page. Wait for the CAPTCHA to appear. Click the 'I'm not a robot' checkbox. Once the CAPTCHA is solved, fill in the 'Name' field with 'Test User' and the 'Email' field with 'test@example.com'. Then click the Submit button.",
  start_url: 'https://www.google.com/recaptcha/api2/demo',
  metadata: { original_task_id: 'captcha-e2e-1' },
}

// ── Helpers ────────────────────────────────────────────────────────────

function log(msg: string) {
  console.log(`[captcha-e2e] ${msg}`)
}

function fail(msg: string): never {
  console.error(`\n[FAIL] ${msg}`)
  process.exit(1)
}

function pass(msg: string) {
  console.log(`\n[PASS] ${msg}`)
}

function preflight() {
  if (!process.env.NOPECHA_API_KEY) {
    fail('NOPECHA_API_KEY env var not set')
  }
  if (!process.env.FIREWORKS_API_KEY) {
    fail('FIREWORKS_API_KEY env var not set — needed for the agent LLM')
  }
  const extDir = join(HERE, '../../extensions/nopecha')
  if (!existsSync(join(extDir, 'manifest.json'))) {
    fail(`NopeCHA extension not found at ${extDir}`)
  }
}

// ── Main ──────────────────────────────────────────────────────────────

async function main() {
  preflight()

  const config = EvalConfigSchema.parse(EVAL_CONFIG)
  const task = TaskSchema.parse(TASK)
  const taskDir = join(OUTPUT_DIR, task.query_id)

  if (existsSync(taskDir)) {
    rmSync(taskDir, { recursive: true, force: true })
  }

  const captcha = config.captcha
  if (!captcha) fail('captcha config block missing')
  const apiKey = process.env[captcha.api_key_env]
  if (!apiKey) fail(`${captcha.api_key_env} env var is empty`)
  BrowserOSAppManager.patchNopechaApiKey(apiKey)

  const app = new BrowserOSAppManager(
    0,
    {
      cdp: config.browseros.base_cdp_port,
      server: config.browseros.base_server_port,
      extension: config.browseros.base_extension_port,
    },
    config.browseros.load_extensions,
    config.browseros.headless,
  )

  try {
    log('Starting BrowserOS stack (headed + NopeCHA extension)...')
    await app.restart()
    log(`BrowserOS ready at ${app.getServerUrl()}`)

    const runConfig = {
      ...config,
      browseros: { ...config.browseros, server_url: app.getServerUrl() },
    }

    const executor = createTaskExecutor(runConfig, OUTPUT_DIR, null)
    log(`Running task: ${task.query_id}`)
    log(`  start_url: ${task.start_url}`)

    const result = await executor.execute(task)

    log(`\nTask status: ${result.status}`)

    if (result.status === 'failed') {
      const err = 'error' in result ? result.error : null
      fail(`Task failed: ${err?.message ?? 'unknown error'}`)
    }

    const metadataPath = join(taskDir, 'metadata.json')
    if (!existsSync(metadataPath)) fail('metadata.json not found')
    const metadata = JSON.parse(readFileSync(metadataPath, 'utf-8'))
    log(`  Duration: ${metadata.total_duration_ms}ms`)
    log(`  Steps: ${metadata.total_steps}`)
    log(`  Termination: ${metadata.termination_reason}`)

    const screenshotDir = join(taskDir, 'screenshots')
    const screenshots = existsSync(screenshotDir)
      ? readdirSync(screenshotDir).filter((f) => f.endsWith('.png'))
      : []
    log(`  Screenshots: ${screenshots.length}`)
    if (screenshots.length === 0) fail('No screenshots captured')

    pass(
      `${screenshots.length} screenshots, ${metadata.total_steps} steps, ${metadata.total_duration_ms}ms`,
    )
  } finally {
    log('Shutting down BrowserOS...')
    await app.killApp()
  }
}

main().catch((err) => {
  console.error(err)
  process.exit(1)
})
