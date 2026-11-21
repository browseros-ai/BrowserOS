import { execFileSync } from 'node:child_process'
import { join, resolve } from 'node:path'

import {
  type BuildTarget,
  type CompiledServerBinary,
  runCommand,
  type TargetId,
} from '@browseros/build-server-tools'

const CROSS_TOOL_VERSION = '0.23.0'
const LINUX_GLIBC_VERSION = '2.17'

/** Cargo package and executable names before the resource packager renames the binary. */
interface RustBinary {
  packageName: string
  binaryName: string
}

interface RustBuildSpec {
  rustTarget: string
  buildTarget: string
  driver: 'cargo' | 'zigbuild' | 'xwin'
  binaryExtension: '' | '.exe'
}

interface ToolRequirement {
  commands: string[]
  install: string
}

export interface RustToolchainProbe {
  platform: NodeJS.Platform
  findCommand: (command: string) => string | undefined
  hasUsableXcode: () => boolean
  installedRustTargets: () => Promise<ReadonlySet<string>>
}

export interface RustBuildInvocation {
  args: string[]
  binaryName: string
  rustTarget: string
}

const RUST_BUILD_SPECS: Record<TargetId, RustBuildSpec> = {
  'linux-x64': {
    rustTarget: 'x86_64-unknown-linux-gnu',
    buildTarget: `x86_64-unknown-linux-gnu.${LINUX_GLIBC_VERSION}`,
    driver: 'zigbuild',
    binaryExtension: '',
  },
  'linux-arm64': {
    rustTarget: 'aarch64-unknown-linux-gnu',
    buildTarget: `aarch64-unknown-linux-gnu.${LINUX_GLIBC_VERSION}`,
    driver: 'zigbuild',
    binaryExtension: '',
  },
  'windows-x64': {
    rustTarget: 'x86_64-pc-windows-msvc',
    buildTarget: 'x86_64-pc-windows-msvc',
    driver: 'xwin',
    binaryExtension: '.exe',
  },
  'darwin-arm64': {
    rustTarget: 'aarch64-apple-darwin',
    buildTarget: 'aarch64-apple-darwin',
    driver: 'cargo',
    binaryExtension: '',
  },
  'darwin-x64': {
    rustTarget: 'x86_64-apple-darwin',
    buildTarget: 'x86_64-apple-darwin',
    driver: 'cargo',
    binaryExtension: '',
  },
}

const defaultProbe: RustToolchainProbe = {
  platform: process.platform,
  findCommand: (command) => Bun.which(command) ?? undefined,
  hasUsableXcode: () => {
    try {
      execFileSync('xcrun', ['--find', 'clang'], { stdio: 'ignore' })
      return true
    } catch {
      return false
    }
  },
  installedRustTargets: async () => {
    const output = execFileSync('rustup', ['target', 'list', '--installed'], {
      encoding: 'utf8',
    })
    return new Set(output.split(/\r?\n/).filter(Boolean))
  },
}

export function rustBuildInvocation(
  target: BuildTarget,
  binary: RustBinary,
): RustBuildInvocation {
  const spec = RUST_BUILD_SPECS[target.id]
  const build =
    spec.driver === 'cargo'
      ? ['build']
      : spec.driver === 'zigbuild'
        ? ['zigbuild']
        : ['xwin', 'build']
  return {
    args: [
      ...build,
      '--release',
      '--locked',
      '--target',
      spec.buildTarget,
      '-p',
      binary.packageName,
      '--bin',
      binary.binaryName,
    ],
    binaryName: `${binary.binaryName}${spec.binaryExtension}`,
    rustTarget: spec.rustTarget,
  }
}

function toolRequirements(targets: BuildTarget[]): ToolRequirement[] {
  const requirements: ToolRequirement[] = [
    {
      commands: ['cargo', 'rustup'],
      install: 'brew install rustup && rustup-init',
    },
    { commands: ['cmake'], install: 'brew install cmake' },
    { commands: ['zip'], install: 'brew install zip' },
    { commands: ['xcrun'], install: 'xcode-select --install' },
  ]
  if (targets.some((target) => target.os === 'linux')) {
    requirements.push(
      { commands: ['zig'], install: 'brew install zig' },
      {
        commands: ['cargo-zigbuild'],
        install: `cargo install --locked cargo-zigbuild --version ${CROSS_TOOL_VERSION}`,
      },
    )
  }
  if (targets.some((target) => target.os === 'windows')) {
    requirements.push(
      {
        commands: ['cargo-xwin'],
        install: `cargo install --locked cargo-xwin --version ${CROSS_TOOL_VERSION}`,
      },
      {
        commands: ['clang-cl', 'lld-link', 'llvm-lib'],
        install: 'brew install llvm; add "$(brew --prefix llvm)/bin" to PATH',
      },
      { commands: ['nasm'], install: 'brew install nasm' },
    )
  }
  return requirements
}

export async function preflightRustBuild(
  targets: BuildTarget[],
  label: string,
  probe: RustToolchainProbe = defaultProbe,
): Promise<void> {
  if (probe.platform !== 'darwin') {
    throw new Error(`The local ${label} builder requires macOS`)
  }

  const missingTools = toolRequirements(targets).flatMap((requirement) => {
    const commands = requirement.commands.filter(
      (command) => !probe.findCommand(command),
    )
    return commands.length > 0 ? [{ ...requirement, commands }] : []
  })
  const missingXcode =
    probe.findCommand('xcrun') !== undefined && !probe.hasUsableXcode()
  const hasRustup = probe.findCommand('rustup') !== undefined
  let installedTargets: ReadonlySet<string> = new Set()
  if (hasRustup) {
    try {
      installedTargets = await probe.installedRustTargets()
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      throw new Error(`Could not query installed Rust targets: ${message}`)
    }
  }
  const missingTargets = hasRustup
    ? [
        ...new Set(
          targets.map((target) => RUST_BUILD_SPECS[target.id].rustTarget),
        ),
      ]
        .filter((target) => !installedTargets.has(target))
        .sort()
    : []

  if (
    missingTools.length === 0 &&
    missingTargets.length === 0 &&
    !missingXcode
  ) {
    return
  }

  const lines = [`Missing ${label} build prerequisites:`]
  for (const requirement of missingTools) {
    lines.push(`- ${requirement.commands.join(', ')}: ${requirement.install}`)
  }
  if (missingXcode) {
    lines.push('- Xcode Command Line Tools: xcode-select --install')
  }
  if (missingTargets.length > 0) {
    lines.push(`- Rust targets: rustup target add ${missingTargets.join(' ')}`)
  }
  throw new Error(lines.join('\n'))
}

/**
 * Builds a Cargo executable for each requested platform. The caller retains
 * ownership of product-specific validation and resource staging.
 */
export async function compileRustBinaries(options: {
  binary: RustBinary
  label: string
  targets: BuildTarget[]
  env: NodeJS.ProcessEnv
  validate?: (binaryPath: string) => Promise<void>
}): Promise<CompiledServerBinary[]> {
  const { binary, label, targets, env, validate } = options
  await preflightRustBuild(targets, label)

  const rootDir = process.cwd()
  const cargoTargetDir = resolve(rootDir, env.CARGO_TARGET_DIR ?? 'target')
  const compiled: CompiledServerBinary[] = []

  for (const target of targets) {
    const invocation = rustBuildInvocation(target, binary)
    await runCommand('cargo', invocation.args, env, rootDir)
    const binaryPath = join(
      cargoTargetDir,
      invocation.rustTarget,
      'release',
      invocation.binaryName,
    )
    await validate?.(binaryPath)
    compiled.push({ target, binaryPath })
  }

  return compiled
}
