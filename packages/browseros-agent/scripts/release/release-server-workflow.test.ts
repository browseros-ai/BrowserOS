import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { browserosServerBuildProduct } from '../build/server/descriptor'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-server.yml'),
  'utf8',
)
const shellVersionPlaceholder = '$' + '{VERSION}'
const shellTargetPlaceholder = '$' + '{target}'
const shellAssetsPlaceholder = '$' + '{assets[@]}'
const publishOtaIf = '$' + '{{ inputs.publish_ota == true }}'
const releaseTagOutput = '$' + '{{ steps.release.outputs.tag }}'
const expectedBumpBranch = `chore-bump-server-v${shellVersionPlaceholder}`

function reflectVersionStep(): string {
  const start = workflow.indexOf('- name: Reflect version on main via PR')
  expect(start).toBeGreaterThanOrEqual(0)
  return workflow.slice(start)
}

function generateReleaseNotesStep(): string {
  const start = workflow.indexOf('- name: Generate release notes')
  const end = workflow.indexOf('- name: Create GitHub release')
  expect(start).toBeGreaterThanOrEqual(0)
  expect(end).toBeGreaterThan(start)
  return workflow.slice(start, end)
}

function createGithubReleaseStep(): string {
  const start = workflow.indexOf('- name: Create GitHub release')
  const end = workflow.indexOf('  build-publish:')
  expect(start).toBeGreaterThanOrEqual(0)
  expect(end).toBeGreaterThan(start)
  return workflow.slice(start, end)
}

describe('release-server workflow', () => {
  it('exposes workflow_call ref and publish_ota inputs', () => {
    expect(workflow).toContain('workflow_call:')
    expect(workflow).toContain('ref:')
    expect(workflow).toContain('publish_ota:')
    expect(workflow).toContain('default: false')
  })

  it('uses a flat branch for post-release version bump PRs', () => {
    const step = reflectVersionStep()
    const branch = step.match(/^\s*BRANCH="([^"]+)"$/m)?.[1]

    expect(branch).toBe(expectedBumpBranch)
    expect(branch?.startsWith('release/')).toBe(false)
  })

  it('fails visibly when post-release version reflection fails', () => {
    const step = reflectVersionStep()

    expect(step).not.toContain('continue-on-error: true')
    expect(step).toContain('GITHUB_STEP_SUMMARY')
    expect(step).toContain('::error::')
  })

  it('creates a missing PR when the remote bump branch already exists', () => {
    const step = reflectVersionStep()

    expect(step).toContain('gh pr list --state open --head "$BRANCH"')
    expect(step).toContain(
      'Branch $BRANCH already exists without an open bump PR; creating it.',
    )
    expect(step).toContain('create_reflection_pr')
  })

  it('publishes all server resource zips to the consumer R2 prefix', () => {
    expect(browserosServerBuildProduct.env.defaultR2UploadPrefix).toBe(
      'artifacts/server',
    )
    expect(browserosServerBuildProduct.env.defaultR2DownloadPrefix).toBe(
      'artifacts/vendor',
    )
    expect(workflow).toContain(
      'bun scripts/build/server.ts --target=all --upload',
    )
    expect(workflow).not.toContain('.env.production')
    expect(workflow).toContain(
      'targets=(darwin-arm64 darwin-x64 linux-arm64 linux-x64 windows-x64)',
    )
    expect(workflow).toContain(
      `artifacts/server/latest/browseros-server-resources-${shellTargetPlaceholder}.zip`,
    )
  })

  it('attaches built zips to the GitHub release and keeps OTA opt-in', () => {
    expect(workflow).toContain(
      `gh release upload "$RELEASE_TAG" "${shellAssetsPlaceholder}" --clobber`,
    )
    expect(workflow).toContain(`if: ${publishOtaIf}`)
    expect(workflow).toContain(
      'uv run browseros ota server release --version "$VERSION" --channel alpha --product browseros',
    )
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
