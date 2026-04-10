import { mkdirSync, rmSync } from 'node:fs'
import { join } from 'node:path'

import { log } from '../log'
import { wasmBinaryPlugin } from '../plugins/wasm-binary'
import { runCommand } from './command'
import type { BuildTarget, CompiledServerBinary } from './types'

const DIST_PROD_ROOT = 'dist/prod/server'
const TMP_ROOT = join(DIST_PROD_ROOT, '.tmp')
const BUNDLE_DIR = join(TMP_ROOT, 'bundle')
const BUNDLE_ENTRY = join(BUNDLE_DIR, 'index.js')
const BINARIES_DIR = join(TMP_ROOT, 'binaries')

function compiledBinaryPath(target: BuildTarget): string {
  return join(
    BINARIES_DIR,
    `browseros-server-${target.id}${target.os === 'windows' ? '.exe' : ''}`,
  )
}

async function bundleServer(
  envVars: Record<string, string>,
  version: string,
): Promise<void> {
  rmSync(BUNDLE_DIR, { recursive: true, force: true })
  mkdirSync(BUNDLE_DIR, { recursive: true })

  const result = await Bun.build({
    entrypoints: ['apps/server/src/index.ts'],
    outdir: BUNDLE_DIR,
    target: 'bun',
    minify: true,
    define: {
      ...Object.fromEntries(
        Object.entries(envVars).map(([key, value]) => [
          `process.env.${key}`,
          JSON.stringify(value),
        ]),
      ),
      __BROWSEROS_VERSION__: JSON.stringify(version),
    },
    external: ['node-pty'],
    plugins: [wasmBinaryPlugin()],
  })

  if (!result.success) {
    const error = result.logs.map((entry) => String(entry)).join('\n')
    throw new Error(`Failed to bundle server:\n${error}`)
  }
}

async function compileTarget(
  target: BuildTarget,
  env: NodeJS.ProcessEnv,
  ci: boolean,
): Promise<string> {
  const binaryPath = compiledBinaryPath(target)
  const args = [
    'build',
    '--compile',
    BUNDLE_ENTRY,
    '--outfile',
    binaryPath,
    `--target=${target.bunTarget}`,
    '--external=node-pty',
  ]
  await runCommand('bun', args, env)

  if (target.os === 'windows') {
    if (ci) {
      log.warn('Skipping Windows exe metadata patching in CI mode')
    } else {
      await runCommand(
        'bun',
        ['scripts/patch-windows-exe.ts', binaryPath],
        process.env,
      )
    }
  }

  return binaryPath
}

export async function compileServerBinaries(
  targets: BuildTarget[],
  envVars: Record<string, string>,
  processEnv: NodeJS.ProcessEnv,
  version: string,
  options?: { ci?: boolean },
): Promise<CompiledServerBinary[]> {
  const ci = options?.ci ?? false
  rmSync(TMP_ROOT, { recursive: true, force: true })
  mkdirSync(BINARIES_DIR, { recursive: true })
  await bundleServer(envVars, version)

  const compiled: CompiledServerBinary[] = []
  for (const target of targets) {
    const binaryPath = await compileTarget(target, processEnv, ci)
    compiled.push({ target, binaryPath })
  }

  rmSync(BUNDLE_DIR, { recursive: true, force: true })
  return compiled
}

export function getDistProdRoot(): string {
  return DIST_PROD_ROOT
}
