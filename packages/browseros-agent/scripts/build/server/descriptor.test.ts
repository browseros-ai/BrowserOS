import { describe, expect, it } from 'bun:test'
import {
  getTargetRules,
  loadManifest,
  resolveTargets,
} from '@browseros/build-server-tools'

import { rustBuildInvocation } from '../rust-compiler'
import {
  browserosServerBinary,
  browserosServerBuildProduct,
} from './descriptor'

describe('BrowserOS Rust server resources', () => {
  it('uses the crate manifest and keeps the released archive identity', () => {
    expect(browserosServerBuildProduct).toMatchObject({
      versionSource: {
        type: 'cargo-toml',
        path: 'crates/browseros-server/Cargo.toml',
      },
      archiveBaseName: 'browseros-server-resources',
      stagedBinaryBaseName: 'browseros_server',
      env: { defaultR2UploadPrefix: 'artifacts/server' },
    })
  })

  it('builds the Rust binary and stages Bun for every shipped target', () => {
    const manifest = loadManifest(
      browserosServerBuildProduct.defaultManifestPath,
    )
    for (const target of resolveTargets('all')) {
      const extension = target.os === 'windows' ? '.exe' : ''
      const invocation = rustBuildInvocation(target, browserosServerBinary)
      expect(invocation.args.slice(-4)).toEqual([
        '-p',
        'browseros-server',
        '--bin',
        'browseros_server',
      ])
      expect(invocation.binaryName).toBe(`browseros_server${extension}`)
      expect(getTargetRules(manifest, target)).toEqual([
        expect.objectContaining({
          destination: `resources/bin/third_party/bun${extension}`,
          executable: true,
        }),
      ])
    }
  })
})
