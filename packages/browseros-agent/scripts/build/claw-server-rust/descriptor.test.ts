import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import {
  getTargetRules,
  loadBuildConfig,
  loadManifest,
  parseBuildArgs,
  resolveTargets,
} from '@browseros/build-server-tools'

import { clawServerRustBuildProduct } from './descriptor'

const agentRoot = resolve(import.meta.dir, '../../..')

describe('BrowserClaw Rust build descriptor', () => {
  it('matches the release artifact and R2 contract', () => {
    expect(clawServerRustBuildProduct).toMatchObject({
      packageDir: 'apps/claw-server-rust',
      versionSource: {
        type: 'cargo-toml',
        path: 'apps/claw-server-rust/Cargo.toml',
      },
      distRoot: 'dist/prod/claw-server-rust',
      stagedBinaryBaseName: 'browseros-claw-server',
      archiveBaseName: 'browseros-claw-server-rust-resources',
      includeArtifactIdentity: true,
      archiveFilesOnly: true,
      env: {
        requiredInlineEnvKeys: ['CLAW_POSTHOG_KEY'],
        defaultR2UploadPrefix: 'claw-server-rust/prod-resources',
      },
    })
  })

  it('reads the canonical Cargo version and supplies a CI-only telemetry key', () => {
    const config = loadBuildConfig(agentRoot, clawServerRustBuildProduct, {
      ci: true,
    })
    const cargo = Bun.TOML.parse(
      readFileSync(
        resolve(agentRoot, 'apps/claw-server-rust/Cargo.toml'),
        'utf8',
      ),
    ) as { package: { version: string } }

    expect(config.version).toBe(cargo.package.version)
    expect(config.envVars.CLAW_POSTHOG_KEY).toBe('phc_browseros_ci')
  })

  it('defaults to all five uploads and stages only the canonical skill', () => {
    const args = parseBuildArgs([], clawServerRustBuildProduct)
    const manifest = loadManifest(
      resolve(agentRoot, clawServerRustBuildProduct.defaultManifestPath),
    )

    expect(args.upload).toBe(true)
    expect(args.targets.map((target) => target.id)).toEqual(
      resolveTargets('all').map((target) => target.id),
    )
    for (const target of args.targets) {
      expect(
        clawServerRustBuildProduct.expectedArtifactFiles?.(target),
      ).toEqual([
        `resources/bin/browseros-claw-server${target.os === 'windows' ? '.exe' : ''}`,
        'resources/skills/browserclaw/SKILL.md',
      ])
      expect(getTargetRules(manifest, target)).toEqual([
        {
          name: 'BrowserOS neo skill',
          source: {
            type: 'local',
            path: 'resources/skills/browserclaw/SKILL.md',
          },
          destination: 'resources/skills/browserclaw/SKILL.md',
          executable: false,
          recursive: false,
        },
      ])
    }
  })

  it('is exposed as the production claw server package command', () => {
    const packageJson = JSON.parse(
      readFileSync(resolve(agentRoot, 'package.json'), 'utf8'),
    ) as { scripts: Record<string, string> }

    expect(packageJson.scripts['build:claw-server']).toBe(
      'FORCE_COLOR=1 bun scripts/build/claw-server-rust.ts --target=all',
    )
    expect(packageJson.scripts['build:claw-server:native']).toBe(
      'cargo build --release -p claw-server-rust',
    )
  })
})
