import { readFile } from 'node:fs/promises'
import type { ProductCompiler } from '@browseros/build-server-tools'

import { compileRustBinaries } from '../rust-compiler'

export const clawServerBinary = {
  packageName: 'claw-server',
  binaryName: 'browseros-claw-server-rs',
}

const VERSION_MARKER_PREFIX = 'browseros-claw-server-version='
const POSTHOG_KEY_MARKER_PREFIX = 'browseros-claw-posthog-key='
const PLAYWRIGHT_RUN_MARKER_PREFIX = 'browseros-claw-playwright-run='
const MARKER_SUFFIX = ';'

export async function validateCompiledBinary(
  binaryPath: string,
  version: string,
  posthogKey: string,
  playwrightRun?: string,
): Promise<void> {
  if (posthogKey.length === 0) {
    throw new Error('CLAW_POSTHOG_KEY is required')
  }
  const binary = await readFile(binaryPath)
  const encodedPosthogKey = Buffer.from(posthogKey).toString('hex')
  const posthogKeyMarker = Buffer.from(
    `${POSTHOG_KEY_MARKER_PREFIX}${encodedPosthogKey}${MARKER_SUFFIX}`,
  )
  if (!binary.includes(posthogKeyMarker)) {
    throw new Error(
      `Compiled BrowserClaw server does not contain CLAW_POSTHOG_KEY: ${binaryPath}`,
    )
  }
  const versionMarker = Buffer.from(
    `${VERSION_MARKER_PREFIX}${version}${MARKER_SUFFIX}`,
  )
  if (!binary.includes(versionMarker)) {
    throw new Error(
      `Compiled BrowserClaw server does not contain version ${version}: ${binaryPath}`,
    )
  }
  const value = playwrightRun ?? ''
  const expectedPlaywrightRun = /^\p{White_Space}*$/u.test(value) ? '0' : value
  if (expectedPlaywrightRun !== '0' && expectedPlaywrightRun !== '1') {
    throw new Error('CLAW_PLAYWRIGHT_RUN must be 0, 1, or unset')
  }
  const otherPlaywrightRun = expectedPlaywrightRun === '0' ? '1' : '0'
  if (
    !binary.includes(
      `${PLAYWRIGHT_RUN_MARKER_PREFIX}${expectedPlaywrightRun}${MARKER_SUFFIX}`,
    ) ||
    binary.includes(
      `${PLAYWRIGHT_RUN_MARKER_PREFIX}${otherPlaywrightRun}${MARKER_SUFFIX}`,
    )
  ) {
    throw new Error(
      `Compiled BrowserClaw server does not match CLAW_PLAYWRIGHT_RUN=${expectedPlaywrightRun}: ${binaryPath}`,
    )
  }
}

export const compileClawServerBinaries: ProductCompiler = (
  product,
  targets,
  envVars,
  processEnv,
  version,
) => {
  const posthogKey = envVars.CLAW_POSTHOG_KEY
  if (!posthogKey) {
    throw new Error(`${product.label}: CLAW_POSTHOG_KEY is required`)
  }
  // Verify the mode from the same merged environment Cargo receives.
  const env = { ...processEnv, ...envVars }
  return compileRustBinaries({
    binary: clawServerBinary,
    label: product.label,
    targets,
    env,
    validate: (binaryPath) =>
      validateCompiledBinary(
        binaryPath,
        version,
        posthogKey,
        env.CLAW_PLAYWRIGHT_RUN,
      ),
  })
}
