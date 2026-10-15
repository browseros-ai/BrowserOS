import { describe, expect, it } from 'bun:test'
import { resolve } from 'node:path'

import {
  getTargetRules,
  loadManifest,
  parseBuildArgs,
  resolveTargets,
} from '@browseros/build-server-tools'

import { rustBuildInvocation } from '../claw-server/compiler'
import { serverRustBuildProduct } from './descriptor'

const root = resolve(import.meta.dir, '../../..')

describe('BrowserOS Rust packaging contract', () => {
  it('keeps production publication opt-in and uses the deployed archive identity', () => {
    const args = parseBuildArgs([], serverRustBuildProduct)
    expect(args.upload).toBe(false)
    expect(args.targets).toHaveLength(5)
    expect(parseBuildArgs(['--upload'], serverRustBuildProduct).upload).toBe(
      true,
    )
    expect(serverRustBuildProduct).toMatchObject({
      archiveBaseName: 'browseros-server-resources',
      stagedBinaryBaseName: 'browseros_server',
      versionSource: {
        type: 'cargo-toml',
        path: 'apps/server-rust/Cargo.toml',
      },
      env: { defaultR2UploadPrefix: 'artifacts/server' },
    })
  })

  it('stages the target-specific Bun runtime with embedded migrations omitted', () => {
    const manifest = loadManifest(
      resolve(root, serverRustBuildProduct.defaultManifestPath),
    )
    for (const target of resolveTargets('all')) {
      const suffix = target.os === 'windows' ? '.exe' : ''
      const rules = getTargetRules(manifest, target)
      expect(rules).toHaveLength(1)
      expect(rules[0]).toMatchObject({
        destination: `resources/bin/third_party/bun${suffix}`,
        executable: true,
        source: { type: 'r2' },
      })
      expect(serverRustBuildProduct.expectedArtifactFiles?.(target)).toEqual([
        `resources/bin/browseros_server${suffix}`,
        `resources/bin/third_party/bun${suffix}`,
      ])
    }
  })

  it('selects the Rust BrowserOS package without changing Claw commands', () => {
    for (const target of resolveTargets('all')) {
      const claw = rustBuildInvocation(target)
      const rust = rustBuildInvocation(target, {
        package: 'server-rust',
        bin: 'browseros-server-rs',
      })
      expect(rust.args).toEqual(
        claw.args.map((arg) =>
          arg === 'claw-server'
            ? 'server-rust'
            : arg === 'browseros-claw-server-rs'
              ? 'browseros-server-rs'
              : arg,
        ),
      )
      expect(rust.binaryName).toBe(
        `browseros-server-rs${target.os === 'windows' ? '.exe' : ''}`,
      )
      expect(rust.rustTarget).toBe(claw.rustTarget)
    }
  })
})
