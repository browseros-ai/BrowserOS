import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-claw-server-rust.yml'),
  'utf8',
)
const shellChannelPlaceholder = '$' + '{channel}'
const shellTargetPlaceholder = '$' + '{target}'
const shellAssetsPlaceholder = '$' + '{assets[@]}'
const releaseTagOutput = '$' + '{{ steps.release.outputs.tag }}'

function generateReleaseNotesStep(): string {
  const start = workflow.indexOf('- name: Generate release notes')
  const end = workflow.indexOf('- name: Create GitHub release')
  expect(start).toBeGreaterThanOrEqual(0)
  expect(end).toBeGreaterThan(start)
  return workflow.slice(start, end)
}

function createGithubReleaseStep(): string {
  const start = workflow.indexOf('- name: Create GitHub release')
  const end = workflow.indexOf('  cargo-test:')
  expect(start).toBeGreaterThanOrEqual(0)
  expect(end).toBeGreaterThan(start)
  return workflow.slice(start, end)
}

describe('release-claw-server-rust workflow', () => {
  it('uses the Rust claw tag trigger and workflow_call contract', () => {
    expect(workflow).toContain('name: "Release: BrowserClaw Server (Rust)"')
    expect(workflow).toContain('"claw-server-rust/v*"')
    expect(workflow).toContain('workflow_call:')
    expect(workflow).toContain('ref:')
    expect(workflow).toContain(
      'Release version; defaults to apps/claw-server-rust/Cargo.toml at ref',
    )
    expect(workflow).toContain('required: false')
    expect(workflow).toContain(
      'packages/browseros-agent/scripts/release/prepare-claw-server-rust-release.sh',
    )
  })

  it('uses only GitHub-hosted Rust runners for the five shipped targets', () => {
    for (const runner of [
      'macos-14',
      'ubuntu-24.04-arm',
      'ubuntu-latest',
      'windows-latest',
    ]) {
      expect(workflow).toContain(`runner: ${runner}`)
    }
    for (const target of [
      'darwin-arm64',
      'darwin-x64',
      'linux-arm64',
      'linux-x64',
      'windows-x64',
    ]) {
      expect(workflow).toContain(`target: ${target}`)
    }
    expect(workflow).not.toContain('warp-')
    expect(workflow).not.toContain('WarpBuild')
  })

  it('runs cargo tests before building and avoids TS Wine patching', () => {
    expect(workflow).toContain('cargo test --workspace --locked')
    expect(workflow).toContain(
      'cargo build --release --locked --target "$RUST_TARGET"',
    )
    expect(workflow).not.toContain('wine')
    expect(workflow).not.toContain('patch-windows-exe')
  })

  it('packages and validates artifact-compatible Rust resource zips', () => {
    expect(workflow).toContain(
      'browseros-claw-server-rust-resources-{target}.zip',
    )
    expect(workflow).toContain('"artifact-metadata.json"')
    expect(workflow).toContain('extract_artifact_zip')
    expect(workflow).toContain('resources/bin/browseros-claw-server-rs')
  })

  it('uses matching artifact actions without unused Python dependencies', () => {
    expect(workflow).toContain('uses: actions/upload-artifact@v7')
    expect(workflow).toContain('uses: actions/download-artifact@v7')
    expect(workflow).toContain('uses: astral-sh/setup-uv@v8.2.0')
    expect(workflow).toContain('uv run --project packages/browseros python')
    expect(workflow).not.toContain('Install Python validation dependencies')
    expect(workflow).not.toContain('pip install ./packages/browseros')
    expect(workflow).not.toContain('.venv-bos-build')
    expect(workflow).not.toContain('pyyaml')
  })

  it('publishes versioned and latest zips to the Rust R2 prefix', () => {
    expect(workflow).toContain('claw-server-rust/prod-resources')
    expect(workflow).toContain(
      `claw-server-rust/prod-resources/${shellChannelPlaceholder}/$(basename "$file")`,
    )
    expect(workflow).toContain(
      `https://cdn.browseros.com/claw-server-rust/prod-resources/latest/browseros-claw-server-rust-resources-${shellTargetPlaceholder}.zip`,
    )
    expect(workflow).not.toContain('claw-server/prod-resources')
  })

  it('attaches all five built zips to the GitHub release', () => {
    expect(workflow).toContain(
      `gh release upload "$RELEASE_TAG" "${shellAssetsPlaceholder}" --clobber`,
    )
    expect(workflow).toContain('Expected 5 Rust server resource zips')
  })

  it('caps generated changelogs before create and edit consume release notes', () => {
    const step = generateReleaseNotesStep()
    const createStep = createGithubReleaseStep()

    expect(step).toContain(`RELEASE_TAG: ${releaseTagOutput}`)
    expect(step).toContain('CHANGELOG_FILE="/tmp/release-changelog.md"')
    expect(step).toContain('NOTES_FILE="/tmp/release-notes.md"')
    expect(step).toContain(
      'node packages/browseros-agent/scripts/release/cap-release-changelog.mjs',
    )
    expect(step).toContain('--max-entries 15')
    expect(step).toContain('--previous-tag "$PREVIOUS_TAG"')
    expect(step).toContain('--release-tag "$RELEASE_TAG"')
    expect(
      createStep.match(/--notes-file \/tmp\/release-notes\.md/g),
    ).toHaveLength(2)
  })
})
