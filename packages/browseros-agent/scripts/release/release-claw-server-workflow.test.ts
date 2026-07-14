import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { clawOnboardBuildProduct } from '../build/claw-onboard/descriptor'
import { clawServerBuildProduct } from '../build/claw-server/descriptor'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-claw-server.yml'),
  'utf8',
)
const shellVersionPlaceholder = '$' + '{VERSION}'
const shellTargetPlaceholder = '$' + '{target}'
const shellServerAssetsPlaceholder = '$' + '{server_assets[@]}'
const publishOtaIf = '$' + '{{ inputs.publish_ota == true }}'
const releaseTagOutput = '$' + '{{ steps.release.outputs.tag }}'
const expectedBumpBranch = `chore-bump-claw-server-v${shellVersionPlaceholder}`

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

describe('release-claw-server workflow', () => {
  it('uses the claw tag trigger and workflow_call contract', () => {
    expect(workflow).toContain('"claw-server/v*"')
    expect(workflow).toContain('workflow_call:')
    expect(workflow).toContain('ref:')
    expect(workflow).toContain('publish_ota:')
  })

  it('publishes claw-server and claw-onboard resources to their consumer prefixes', () => {
    expect(clawServerBuildProduct.env.defaultR2UploadPrefix).toBe(
      'claw-server/prod-resources',
    )
    expect(clawOnboardBuildProduct.env.defaultR2UploadPrefix).toBe(
      'claw-onboard/prod-resources',
    )
    expect(workflow).toContain(
      'bun scripts/build/claw-server.ts --target=all --upload',
    )
    expect(workflow).toContain('bun scripts/build/claw-onboard.ts --upload')
    expect(workflow).toContain(
      `claw-server/prod-resources/latest/browseros-claw-server-resources-${shellTargetPlaceholder}.zip`,
    )
    expect(workflow).toContain(
      'claw-onboard/prod-resources/latest/browseros-claw-onboard-resources.zip',
    )
  })

  it('attaches all built zips to the GitHub release and keeps OTA opt-in', () => {
    expect(workflow).toContain(
      `gh release upload "$RELEASE_TAG" "${shellServerAssetsPlaceholder}" "$onboard_asset" --clobber`,
    )
    expect(workflow).toContain(`if: ${publishOtaIf}`)
    expect(workflow).toContain(
      'uv run browseros ota server release --version "$VERSION" --channel alpha --product browserclaw',
    )
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
