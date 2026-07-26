import { mkdirSync, rmSync } from 'node:fs'
import { basename, extname, join } from 'node:path'
import { runCommand } from './command'
import { log } from './log'
import type {
  BuildProductDescriptor,
  BuildTarget,
  CompiledServerBinary,
} from './types'

function tmpRoot(product: BuildProductDescriptor): string {
  return join(product.distRoot, '.tmp')
}

function bundleDir(product: BuildProductDescriptor): string {
  return join(tmpRoot(product), 'bundle')
}

function binariesDir(product: BuildProductDescriptor): string {
  return join(tmpRoot(product), 'binaries')
}

function bundledEntrypoint(product: BuildProductDescriptor): string {
  const sourceBaseName = basename(product.entrypoint)
  const extension = extname(sourceBaseName)
  return join(
    bundleDir(product),
    `${sourceBaseName.slice(0, -extension.length)}.js`,
  )
}

export function compiledBinaryPath(
  product: BuildProductDescriptor,
  target: BuildTarget,
): string {
  return join(
    binariesDir(product),
    `${product.rawBinaryBaseName}-${target.id}${target.os === 'windows' ? '.exe' : ''}`,
  )
}

async function bundleProduct(
  product: BuildProductDescriptor,
  envVars: Record<string, string>,
  version: string,
): Promise<void> {
  rmSync(bundleDir(product), { recursive: true, force: true })
  mkdirSync(bundleDir(product), { recursive: true })

  const result = await Bun.build({
    entrypoints: [product.entrypoint],
    outdir: bundleDir(product),
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
    external: [...(product.bundle?.external ?? [])],
    plugins: product.bundle?.plugins ?? [],
  })

  if (!result.success) {
    const error = result.logs.map((entry) => String(entry)).join('\n')
    throw new Error(`Failed to bundle ${product.label}:\n${error}`)
  }
}

async function compileTarget(
  product: BuildProductDescriptor,
  target: BuildTarget,
  env: NodeJS.ProcessEnv,
  ci: boolean,
): Promise<string> {
  const binaryPath = compiledBinaryPath(product, target)
  const args = [
    'build',
    '--compile',
    bundledEntrypoint(product),
    '--outfile',
    binaryPath,
    `--target=${target.bunTarget}`,
    ...[...(product.bundle?.external ?? [])].map(
      (external) => `--external=${external}`,
    ),
  ]
  await runCommand('bun', args, env)
  await adHocSignMacBinary(target, binaryPath, env)

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

/** Keeps local macOS artifacts valid until release signing replaces the ad-hoc signature. */
async function adHocSignMacBinary(
  target: BuildTarget,
  binaryPath: string,
  env: NodeJS.ProcessEnv,
): Promise<void> {
  if (target.os !== 'macos') return
  if (process.platform !== 'darwin') {
    log.warn(`Skipping ad-hoc signing for ${target.id} outside macOS`)
    return
  }

  await runCommand(
    'codesign',
    ['--force', '--sign', '-', '--timestamp=none', binaryPath],
    env,
  )
}

/** Bundles once, then compiles the bundled entrypoint for each requested target. */
export async function compileProductBinaries(
  product: BuildProductDescriptor,
  targets: BuildTarget[],
  envVars: Record<string, string>,
  processEnv: NodeJS.ProcessEnv,
  version: string,
  options?: { ci?: boolean },
): Promise<CompiledServerBinary[]> {
  const ci = options?.ci ?? false
  rmSync(tmpRoot(product), { recursive: true, force: true })
  mkdirSync(binariesDir(product), { recursive: true })
  await bundleProduct(product, envVars, version)

  const compiled: CompiledServerBinary[] = []
  try {
    for (const target of targets) {
      const binaryPath = await compileTarget(product, target, processEnv, ci)
      compiled.push({ target, binaryPath })
    }
  } finally {
    rmSync(bundleDir(product), { recursive: true, force: true })
  }

  return compiled
}
