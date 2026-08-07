import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-claw-onboard.yml'),
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

describe('release-claw-onboard workflow', () => {
  it('exposes the reusable build/finalize interface and outputs', () => {
    expect(workflow).toContain('name: "Release: BrowserOS neo Onboarding"')
    expect(workflow).toContain('"claw-onboard/v*"')
    expect(workflow).toContain('browseros release component resolve')
    expect(workflow).toContain('--component claw-onboard')
    expect(workflow).not.toContain('prepare-claw-onboard-release.sh')
    const call = section('  workflow_call:', '\npermissions:')
    expect(call).toContain('mode:')
    expect(call).toContain('default: "build"')
    expect(call).toContain('defer_finalize:')
    expect(call).toContain('default: false')
    expect(call).toContain(`value: ${dollar}{{ jobs.prepare.outputs.version }}`)
    expect(call).toContain(`value: ${dollar}{{ jobs.prepare.outputs.tag }}`)
    expect(call).toContain(
      `value: ${dollar}{{ jobs.prepare.outputs.release_sha }}`,
    )
  })

  it('reserves a private draft before building', () => {
    const prepare = section('  prepare:', '  build-publish:')
    expect(prepare).toContain('browseros release component resolve')
    expect(prepare).toContain('--draft')
    expect(prepare).not.toContain('git tag -a')
    expect(prepare).not.toContain('--draft=false')
  })

  it('checks public allocations under the component lock before mutations', () => {
    const prepare = section('  prepare:', '  build-publish:')
    expect(workflow).toContain('group: release-claw-onboard')
    expect(prepare.indexOf('Setup uv')).toBeLessThan(
      prepare.indexOf('Resolve release'),
    )
    expect(prepare.indexOf('Resolve release')).toBeLessThan(
      prepare.indexOf('Reserve private draft'),
    )
  })

  it('uploads only the immutable onboarding version and attaches it', () => {
    const build = section('  build-publish:', '  finalize:')
    expect(build).toContain(
      'bun scripts/build/claw-onboard.ts --upload --versioned-only',
    )
    expect(build).toContain('RELEASE_SHA:')
    expect(build).toContain(
      'dist/prod/claw-onboard/browseros-claw-onboard-resources.zip',
    )
    expect(build).not.toContain('claw-onboard/prod-resources/latest/')
    expect(build).toContain(
      'browseros release component stamp --component claw-onboard',
    )
    expect(build).not.toContain('packageJson.version = version')
    expect(build).toContain(
      'gh release upload "$RELEASE_TAG" "$asset" --clobber',
    )
  })

  it('publishes then moves latest during finalization', () => {
    const finalize = section('  finalize:', '  reflect-version:')
    expect(finalize).toContain('Expected onboarding draft asset')
    expect(finalize).toContain(
      `key="claw-onboard/prod-resources/${dollar}{VERSION}/${dollar}{name}"`,
    )
    expect(finalize).toContain('--arg component "claw-onboard/prod-resources"')
    expect(finalize).toContain('aws s3api get-object')
    expect(finalize).toContain('gh release download "$RELEASE_TAG"')
    expect(finalize).toContain('sha256sum')
    expect(finalize).toContain('Draft asset does not match canonical R2 object')
    expect(finalize).toContain('--metadata-directive COPY')
    const verifyIndex = finalize.indexOf('Verify prepared release')
    const tagIndex = finalize.indexOf('git tag -a "$RELEASE_TAG"')
    const publishIndex = finalize.indexOf('--draft=false')
    const latestIndex = finalize.indexOf('Copy versioned object to latest')
    expect(tagIndex).toBeGreaterThan(verifyIndex)
    expect(publishIndex).toBeGreaterThan(tagIndex)
    expect(latestIndex).toBeGreaterThan(publishIndex)
  })

  it('reflects the version only after finalization', () => {
    const reflection = section('  reflect-version:')
    expect(reflection).toContain('- finalize')
    expect(reflection).toContain('apps/claw-onboard/package.json')
    expect(reflection).toContain('git config user.name "github-actions[bot]"')
    expect(reflection).toContain('gh pr create')
  })
})
