import { afterEach, describe, expect, it } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { resolveTargets } from '@browseros/build-server-tools'

import {
  preflightRustBuild,
  type RustToolchainProbe,
  rustBuildInvocation,
  validateCompiledBinary,
} from './compiler'

describe('BrowserClaw Rust compiler', () => {
  let tempDir: string | null = null

  afterEach(async () => {
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('maps every shipped target to its release cross-compiler and output', () => {
    const invocations = resolveTargets('all').map((target) => ({
      id: target.id,
      ...rustBuildInvocation(target),
    }))

    expect(invocations).toEqual([
      expect.objectContaining({
        id: 'linux-x64',
        rustTarget: 'x86_64-unknown-linux-gnu',
        binaryName: 'browseros-claw-server-rs',
        args: expect.arrayContaining([
          'zigbuild',
          '--target',
          'x86_64-unknown-linux-gnu.2.17',
        ]),
      }),
      expect.objectContaining({
        id: 'linux-arm64',
        rustTarget: 'aarch64-unknown-linux-gnu',
        binaryName: 'browseros-claw-server-rs',
        args: expect.arrayContaining([
          'zigbuild',
          '--target',
          'aarch64-unknown-linux-gnu.2.17',
        ]),
      }),
      expect.objectContaining({
        id: 'windows-x64',
        rustTarget: 'x86_64-pc-windows-msvc',
        binaryName: 'browseros-claw-server-rs.exe',
        args: expect.arrayContaining([
          'xwin',
          'build',
          '--target',
          'x86_64-pc-windows-msvc',
        ]),
      }),
      expect.objectContaining({
        id: 'darwin-arm64',
        rustTarget: 'aarch64-apple-darwin',
        binaryName: 'browseros-claw-server-rs',
        args: expect.arrayContaining([
          'build',
          '--target',
          'aarch64-apple-darwin',
        ]),
      }),
      expect.objectContaining({
        id: 'darwin-x64',
        rustTarget: 'x86_64-apple-darwin',
        binaryName: 'browseros-claw-server-rs',
        args: expect.arrayContaining([
          'build',
          '--target',
          'x86_64-apple-darwin',
        ]),
      }),
    ])
    for (const invocation of invocations) {
      expect(invocation.args).toContain('--release')
      expect(invocation.args).toContain('--locked')
      expect(invocation.args).toContain('claw-server-rust')
      expect(invocation.args).toContain('browseros-claw-server-rs')
    }
  })

  it('accepts a complete macOS cross-compilation toolchain', async () => {
    const targets = resolveTargets('all')
    const probe = fakeProbe({
      targets: new Set(
        targets.map((target) => rustBuildInvocation(target).rustTarget),
      ),
    })

    await expect(preflightRustBuild(targets, probe)).resolves.toBeUndefined()
  })

  it('rejects non-macOS hosts before probing tools', async () => {
    let probed = false
    const probe = fakeProbe({
      platform: 'linux',
      findCommand: () => {
        probed = true
        return '/bin/tool'
      },
    })

    await expect(
      preflightRustBuild(resolveTargets('darwin-arm64'), probe),
    ).rejects.toThrow('requires macOS')
    expect(probed).toBe(false)
  })

  it('reports all missing tools and Rust targets with install commands', async () => {
    const missing = new Set([
      'zig',
      'cargo-zigbuild',
      'cargo-xwin',
      'clang-cl',
      'lld-link',
      'llvm-lib',
      'nasm',
    ])
    const probe = fakeProbe({
      findCommand: (command) =>
        missing.has(command) ? undefined : `/bin/${command}`,
      targets: new Set(['aarch64-apple-darwin']),
    })

    const error = await preflightRustBuild(resolveTargets('all'), probe).catch(
      (caught) => caught as Error,
    )

    expect(error.message).toContain('brew install zig')
    expect(error.message).toContain('brew install llvm')
    expect(error.message).toContain('brew install nasm')
    expect(error.message).toContain(
      'cargo install --locked cargo-zigbuild --version 0.23.0',
    )
    expect(error.message).toContain(
      'cargo install --locked cargo-xwin --version 0.23.0',
    )
    expect(error.message).toContain(
      'rustup target add aarch64-unknown-linux-gnu x86_64-apple-darwin x86_64-pc-windows-msvc x86_64-unknown-linux-gnu',
    )
  })

  it('requires a usable Xcode clang even for cross-only targets', async () => {
    const target = resolveTargets('linux-x64')
    const probe = fakeProbe({
      hasUsableXcode: () => false,
      targets: new Set(['x86_64-unknown-linux-gnu']),
    })

    await expect(preflightRustBuild(target, probe)).rejects.toThrow(
      'Xcode Command Line Tools: xcode-select --install',
    )
  })

  it('validates embedded version and telemetry key without exposing the key', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'claw-rust-compiler-'))
    const binaryPath = join(tempDir, 'server')
    const posthogKey = 'phc_private_test_key'
    const encodedKey = Buffer.from(posthogKey).toString('hex')
    await writeFile(
      binaryPath,
      `binary:browseros-claw-posthog-key=${encodedKey};browseros-claw-server-version=1.2.3;`,
    )

    await expect(
      validateCompiledBinary(binaryPath, '1.2.3', posthogKey),
    ).resolves.toBeUndefined()

    await writeFile(binaryPath, 'binary:browseros-claw-server-version=1.2.3;')
    const error = await validateCompiledBinary(
      binaryPath,
      '1.2.3',
      posthogKey,
    ).catch((caught) => caught as Error)
    expect(error.message).toContain('CLAW_POSTHOG_KEY')
    expect(error.message).not.toContain(posthogKey)

    const longerKey = `${posthogKey}_extra`
    await writeFile(
      binaryPath,
      `binary:browseros-claw-posthog-key=${Buffer.from(longerKey).toString('hex')};browseros-claw-server-version=1.2.3;`,
    )
    await expect(
      validateCompiledBinary(binaryPath, '1.2.3', posthogKey),
    ).rejects.toThrow('CLAW_POSTHOG_KEY')

    await writeFile(
      binaryPath,
      `binary:browseros-claw-posthog-key=${encodedKey};browseros-claw-server-version=1.2.30;`,
    )
    await expect(
      validateCompiledBinary(binaryPath, '1.2.3', posthogKey),
    ).rejects.toThrow('version 1.2.3')
  })
})

function fakeProbe(
  overrides: Partial<RustToolchainProbe> & {
    targets?: ReadonlySet<string>
  } = {},
): RustToolchainProbe {
  return {
    platform: overrides.platform ?? 'darwin',
    findCommand: overrides.findCommand ?? ((command) => `/bin/${command}`),
    hasUsableXcode: overrides.hasUsableXcode ?? (() => true),
    installedRustTargets:
      overrides.installedRustTargets ??
      (async () => overrides.targets ?? new Set()),
  }
}
