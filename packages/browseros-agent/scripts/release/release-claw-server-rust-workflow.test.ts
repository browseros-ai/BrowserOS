import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-claw-server-rust.yml'),
  'utf8',
)
const localStaging = readFileSync(
  resolve(
    repoRoot,
    'packages/browseros-agent/scripts/build/claw-server-rust-local.sh',
  ),
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

describe('release-claw-server-rust workflow', () => {
  it('exposes the reusable build/finalize interface and outputs', () => {
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

  it('reserves only a draft before tests and builds', () => {
    const prepare = section('  prepare:', '  cargo-test:')
    expect(prepare).toContain('gh api --paginate --slurp')
    expect(prepare).toContain('--release-records "$RELEASE_RECORDS"')
    expect(prepare).toContain('--draft')
    expect(prepare).not.toContain('git tag -a')
    expect(prepare).not.toContain('--draft=false')
    expect(workflow.indexOf('  cargo-test:')).toBeGreaterThan(
      workflow.indexOf('  prepare:'),
    )
  })

  it('checks public allocations under the component lock before mutations', () => {
    const prepare = section('  prepare:', '  cargo-test:')
    expect(workflow).toContain('group: release-claw-server-rust')
    expect(prepare.indexOf('Read allocated GitHub releases')).toBeLessThan(
      prepare.indexOf('Resolve release'),
    )
    expect(prepare.indexOf('Resolve release')).toBeLessThan(
      prepare.indexOf('Reserve private draft'),
    )
  })

  it('tests and builds all five shipped targets', () => {
    expect(workflow).toContain('cargo test --workspace --locked')
    expect(workflow).toContain('cargo test --locked -p harness-integrations')
    for (const target of [
      'darwin-arm64',
      'darwin-x64',
      'linux-arm64',
      'linux-x64',
      'windows-x64',
    ]) {
      expect(workflow).toContain(`target: ${target}`)
    }
    expect(workflow).toContain('CLAW_POSTHOG_KEY is required')
    expect(workflow).toContain('Verify stamped binary version')
  })

  it('uploads only immutable version keys and attaches all draft assets', () => {
    const publish = section('  publish-versioned:', '  finalize:')
    expect(publish).toContain('actions/setup-python@v6')
    expect(publish).toContain('python -m pip install "boto3>=1.35.1,<2"')
    expect(publish).not.toContain('pip install --user')
    expect(publish).toContain('Expected 5 Rust server resource zips')
    expect(publish).toContain('IfNoneMatch')
    expect(publish).toContain('status not in {409, 412}')
    expect(publish).toContain('Metadata={')
    expect(publish).toContain('"release-sha": release_sha')
    expect(publish).toContain('"component": "claw-server-rust/prod-resources"')
    expect(publish).toContain('path.write_bytes(existing)')
    expect(publish).toContain(
      'f"claw-server-rust/prod-resources/{version}/{path.name}"',
    )
    expect(publish).not.toContain('claw-server-rust/prod-resources/latest/')
    expect(publish).toContain(
      `gh release upload "$RELEASE_TAG" "${dollar}{assets[@]}" --clobber`,
    )
  })

  it('recovers completed targets before rebuilding missing targets', () => {
    const build = section('  build:', '  publish-versioned:')
    expect(build).toContain('Recover existing immutable target')
    expect(build).toContain('actions/setup-python@v6')
    expect(build).toContain('python -m pip install boto3')
    expect(build).not.toContain('pip install --user boto3')
    expect(build).toContain('Immutable R2 object binding mismatch')
    expect(build).toContain('path.write_bytes(data)')
    expect(build).toContain("steps.recover.outputs.recovered != 'true'")
    expect(build).toContain(
      'steps.package.outputs.zip_path || steps.recover.outputs.zip_path',
    )
  })

  it('moves every latest alias before publishing during finalization', () => {
    const finalize = section('  finalize:', '  publish-ota:')
    expect(finalize).toContain('actions/setup-python@v6')
    expect(finalize).toContain('python -m pip install awscli')
    expect(finalize).not.toContain('pip install --user')
    expect(finalize).toContain('Expected 5 draft assets')
    expect(finalize).toContain(
      '--arg component "claw-server-rust/prod-resources"',
    )
    expect(finalize).toContain('aws s3api get-object')
    expect(finalize).toContain('gh release download "$RELEASE_TAG"')
    expect(finalize).toContain('sha256sum')
    expect(finalize).toContain('Draft asset does not match canonical R2 object')
    expect(finalize).toContain('--metadata-directive COPY')
    const verifyIndex = finalize.indexOf('Verify prepared release')
    const tagIndex = finalize.indexOf('git tag -a "$RELEASE_TAG"')
    const publishIndex = finalize.indexOf('--draft=false')
    const latestIndex = finalize.indexOf('Copy versioned objects to latest')
    expect(tagIndex).toBeGreaterThan(verifyIndex)
    expect(latestIndex).toBeGreaterThan(tagIndex)
    expect(publishIndex).toBeGreaterThan(latestIndex)
  })

  it('packages the canonical BrowserOS skill', () => {
    const build = section('  build:', '  publish-versioned:')
    expect(build).toContain(
      'source_skill = agent / "resources/skills/browserclaw/SKILL.md"',
    )
    expect(build).toContain('shutil.copy2(source_skill, staged_skill)')
    expect(build).toContain('"resources/skills/browserclaw/SKILL.md"')
    expect(localStaging).toContain(
      'source_skill = agent_root / "resources/skills/browserclaw/SKILL.md"',
    )
    expect(localStaging).toContain('shutil.copy2(source_skill, staged_skill)')
  })

  it('gates OTA publication on successful finalization', () => {
    const ota = section('  publish-ota:', '  reflect-version:')
    expect(ota).toContain('- finalize')
    expect(ota).toContain('inputs.publish_ota == true')
    expect(ota).toContain(
      'uv run browseros ota server release --version "$VERSION" --channel alpha --product browserclaw',
    )
  })

  it('reflects the version only after finalization', () => {
    const reflection = section('  reflect-version:')
    expect(reflection).toContain('- finalize')
    expect(reflection).toContain('apps/claw-server-rust/Cargo.toml')
    expect(reflection).toContain('Cargo.lock')
    expect(reflection).toContain('git config user.name "github-actions[bot]"')
    expect(reflection).toContain('gh pr create')
  })
})
