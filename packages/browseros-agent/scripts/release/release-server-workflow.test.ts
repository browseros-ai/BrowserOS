import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { CROSS_TOOL_VERSION } from '../build/claw-server/compiler'
import { serverRustBuildProduct } from '../build/server-rust/descriptor'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-server.yml'),
  'utf8',
)
const dollar = '$'

function section(start: string, end?: string): string {
  const startIndex = workflow.indexOf(start)
  expect(startIndex).toBeGreaterThanOrEqual(0)
  const endIndex = end ? workflow.indexOf(end, startIndex + start.length) : -1
  if (end) expect(endIndex).toBeGreaterThan(startIndex)
  return workflow.slice(startIndex, endIndex === -1 ? undefined : endIndex)
}

describe('release-server workflow', () => {
  it('exposes the reusable build/finalize interface and outputs', () => {
    const call = section('  workflow_call:', '\npermissions:')
    const publishOta = call.slice(
      call.indexOf('      publish_ota:'),
      call.indexOf('    outputs:'),
    )
    expect(call).toContain('mode:')
    expect(call).toContain('default: "build"')
    expect(call).toContain('defer_finalize:')
    expect(call).toContain('default: false')
    expect(call).toContain('version:')
    expect(call).toContain('ref:')
    expect(call).toContain('publish_ota:')
    expect(publishOta).toContain('default: false')
    expect(call).toContain('outputs:')
    expect(call).toContain(`value: ${dollar}{{ jobs.prepare.outputs.version }}`)
    expect(call).toContain(`value: ${dollar}{{ jobs.prepare.outputs.tag }}`)
    expect(call).toContain(
      `value: ${dollar}{{ jobs.prepare.outputs.release_sha }}`,
    )
  })

  it('reserves a private draft without creating a public tag', () => {
    const prepare = section('  prepare:', '  build-publish:')
    expect(prepare).toContain('browseros release component resolve')
    expect(prepare).toContain('--component server')
    expect(prepare).not.toContain('prepare-server-release.sh')
    expect(prepare).toContain('gh release create "$RELEASE_TAG"')
    expect(prepare).toContain('--draft')
    expect(prepare).toContain('--target "$RELEASE_SHA"')
    expect(prepare).not.toContain('git tag -a')
    expect(prepare).not.toContain('--draft=false')
  })

  it('checks public allocations under the component lock before mutations', () => {
    const prepare = section('  prepare:', '  build-publish:')
    expect(workflow).toContain('group: release-server')
    expect(prepare).toContain('browseros release component resolve')
    expect(prepare.indexOf('Setup uv')).toBeLessThan(
      prepare.indexOf('Resolve release'),
    )
    expect(prepare.indexOf('Resolve release')).toBeLessThan(
      prepare.indexOf('Reserve private draft'),
    )
  })

  it('builds and attaches every immutable versioned artifact first', () => {
    const build = section('  build-publish:', '  finalize:')
    expect(serverRustBuildProduct.env.defaultR2UploadPrefix).toBe(
      'artifacts/server',
    )
    expect(serverRustBuildProduct.archiveBaseName).toBe(
      'browseros-server-resources',
    )
    expect(build).toContain(
      'bun scripts/build/server-rust.ts --target=all --upload --versioned-only',
    )
    expect(build).not.toContain('scripts/build/server.ts')
    expect(build).toContain(
      `${serverRustBuildProduct.distRoot}/browseros-server-resources-*.zip`,
    )
    expect(build).toContain('RELEASE_SHA:')
    expect(build).toContain('Expected 5 server resource zips')
    expect(build).toContain(
      `gh release upload "$RELEASE_TAG" "${dollar}{assets[@]}" --clobber`,
    )
    expect(build).not.toContain('artifacts/server/latest/')
    expect(build).toContain(
      'browseros release component stamp --component server',
    )
    expect(build).not.toContain('package["version"] = version')
  })

  it('cross-compiles the Rust server with the packager toolchain', () => {
    const build = section('  build-publish:', '  finalize:')
    expect(build).toContain('runs-on: macos-14')
    for (const triple of [
      'aarch64-apple-darwin',
      'x86_64-apple-darwin',
      'aarch64-unknown-linux-gnu',
      'x86_64-unknown-linux-gnu',
      'x86_64-pc-windows-msvc',
    ]) {
      expect(build).toContain(triple)
    }
    expect(build).toContain(`CROSS_TOOL_VERSION: "${CROSS_TOOL_VERSION}"`)
    expect(build).toContain('cargo install --locked cargo-zigbuild')
    expect(build).toContain('cargo install --locked cargo-xwin')
    expect(build).toContain('ZIG_SHA256:')
    expect(build).not.toContain('mapfile -t')
    expect(build).not.toContain('wine')
  })

  it('finalizes only after verifying assets and versioned objects', () => {
    const finalize = section('  finalize:', '  publish-ota:')
    expect(finalize).toContain('Expected 5 draft assets')
    expect(finalize).toContain(
      `key="artifacts/server/${dollar}{VERSION}/${dollar}{name}"`,
    )
    expect(finalize).toContain('--arg component "artifacts/server"')
    expect(finalize).toContain('aws s3api get-object')
    expect(finalize).toContain('gh release download "$RELEASE_TAG"')
    expect(finalize).toContain('sha256sum')
    expect(finalize).toContain('Draft asset does not match canonical R2 object')
    expect(finalize).toContain('--metadata-directive COPY')
    const verifyIndex = finalize.indexOf('Verify prepared release')
    const tagIndex = finalize.indexOf('git tag -a "$RELEASE_TAG"')
    const publishIndex = finalize.indexOf('--draft=false')
    const latestIndex = finalize.indexOf('Copy versioned objects to latest')
    expect(verifyIndex).toBeGreaterThanOrEqual(0)
    expect(tagIndex).toBeGreaterThan(verifyIndex)
    expect(latestIndex).toBeGreaterThan(tagIndex)
    expect(publishIndex).toBeGreaterThan(latestIndex)
  })

  it('defers finalization for reusable callers and gates side effects on it', () => {
    const finalize = section('  finalize:', '  publish-ota:')
    expect(finalize).toContain("needs.prepare.outputs.mode == 'finalize'")
    expect(finalize).toContain('inputs.defer_finalize != true')
    expect(section('  publish-ota:', '  reflect-version:')).toContain(
      '- finalize',
    )
    expect(section('  reflect-version:')).toContain('- finalize')
  })

  it('reflects the released version into the Rust Cargo package', () => {
    const reflection = section('  reflect-version:')
    expect(reflection).toContain(
      'cargo_toml="packages/browseros-agent/apps/server-rust/Cargo.toml"',
    )
    expect(reflection).toContain('lock="packages/browseros-agent/Cargo.lock"')
    expect(reflection).not.toContain('apps/server/package.json')
  })

  it('merges reflection PRs through the verified release helper', () => {
    const reflection = section('  reflect-version:')
    expect(reflection).toContain('merge-release-pr.sh')
    expect(reflection).toContain('headRefOid')
    expect(reflection).not.toContain('--squash --auto')
  })

  it('publishes and persists standalone alpha releases by default', () => {
    const dispatch = section('  workflow_dispatch:', '  workflow_call:')
    const ota = section('  publish-ota:', '  reflect-version:')
    expect(dispatch).toContain('publish_ota:')
    expect(dispatch).toContain('default: true')
    expect(ota).toContain("github.event_name != 'push'")
    expect(ota).toContain('inputs.publish_ota == true')
    expect(ota).toContain('pull-requests: write')
    expect(ota).toContain('uses: ./.github/workflows/publish-server-ota.yml')
    expect(ota).toContain('product: browseros')
    expect(ota).toContain(
      `version: ${dollar}{{ needs.prepare.outputs.version }}`,
    )
    expect(ota).toContain(
      `release_sha: ${dollar}{{ needs.prepare.outputs.release_sha }}`,
    )
    expect(ota).toContain('secrets: inherit')
    expect(ota).toContain('updates/server/appcast-server.alpha.xml')
    expect(ota).not.toContain('updates/server/appcast-claw-server.alpha.xml')
  })
})
