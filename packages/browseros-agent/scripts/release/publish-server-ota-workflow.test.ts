import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/publish-server-ota.yml'),
  'utf8',
)

function section(start: string, end?: string): string {
  const startIndex = workflow.indexOf(start)
  expect(startIndex).toBeGreaterThanOrEqual(0)
  const endIndex = end ? workflow.indexOf(end, startIndex + start.length) : -1
  if (end) expect(endIndex).toBeGreaterThan(startIndex)
  return workflow.slice(startIndex, endIndex === -1 ? undefined : endIndex)
}

describe('publish-server-ota workflow', () => {
  it('signs every platform on its native runner', () => {
    const build = section('  build-payloads:', '  publish:')
    for (const platform of [
      'darwin_arm64',
      'darwin_x64',
      'linux_arm64',
      'linux_x64',
      'windows_x64',
    ]) {
      expect(build).toContain(`platform: ${platform}`)
    }
    expect(build).toContain('runner: macos-14')
    expect(build).toContain('runner: windows-latest')
    expect(build).toContain('runner: ubuntu-latest')
    expect(build).toContain('Import macOS signing certificate')
    expect(build).toContain('Install SSL.com CodeSignTool')
    expect(build).toContain('MACOS_CERTIFICATE_NAME:')
    expect(build).toContain('ESIGNER_USERNAME:')
    expect(build).toContain('SPARKLE_PRIVATE_KEY:')
    expect(build).toContain('--platform "$PLATFORM"')
    expect(build).toContain('actions/upload-artifact@v7')
  })

  it('assembles all fragments before one guarded live publication', () => {
    const publish = section('  publish:')
    expect(publish).toContain('- build-payloads')
    expect(publish).toContain('actions/download-artifact@v7')
    expect(publish).toContain('pattern: server-ota-fragment-*')
    expect(publish).toContain('ota server assemble-appcast')
    expect(publish).toContain(
      'ota server release-appcast --channel alpha --product "$PRODUCT" --publish',
    )
    expect(publish).toContain('commit-update-snapshot.sh')

    const assembleIndex = publish.indexOf('ota server assemble-appcast')
    const liveIndex = publish.indexOf('ota server release-appcast')
    const snapshotIndex = publish.indexOf('commit-update-snapshot.sh')
    expect(liveIndex).toBeGreaterThan(assembleIndex)
    expect(snapshotIndex).toBeGreaterThan(liveIndex)
  })

  it('accepts only product-owned snapshot inputs from its callers', () => {
    const call = section('  workflow_call:', '\npermissions:')
    expect(call).toContain('product:')
    expect(call).toContain('version:')
    expect(call).toContain('release_sha:')
    expect(call).toContain('snapshot_path:')
    expect(workflow).not.toContain('--channel prod')
  })
})
