import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-claw-onboard.yml'),
  'utf8',
)
const browserClawWorkflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-browserclaw.yml'),
  'utf8',
)

describe('release-claw-onboard workflow', () => {
  it('owns an independent onboarding release contract', () => {
    expect(workflow).toContain('name: "Release: BrowserClaw Onboarding"')
    expect(workflow).toContain('"claw-onboard/v*"')
    expect(workflow).toContain('workflow_call:')
    expect(workflow).toContain(
      'packages/browseros-agent/scripts/release/prepare-claw-onboard-release.sh',
    )
    expect(workflow).not.toContain('apps/claw-server')
    expect(workflow).not.toContain('claw-server/prod-resources')
  })

  it('publishes and attaches only the onboarding resource zip', () => {
    expect(workflow).toContain('bun scripts/build/claw-onboard.ts --upload')
    expect(workflow).toContain(
      'dist/prod/claw-onboard/browseros-claw-onboard-resources.zip',
    )
    expect(workflow).toContain(
      'claw-onboard/prod-resources/latest/browseros-claw-onboard-resources.zip',
    )
    expect(workflow).not.toContain('browseros-claw-server-resources-')
    expect(workflow).not.toContain('wine')
  })

  it('reflects only claw-onboard and its lock entry', () => {
    const reflection = workflow.slice(workflow.indexOf('  reflect-version:'))
    expect(reflection).toContain('apps/claw-onboard/package.json')
    expect(reflection).toContain('"apps\\/claw-onboard"')
    expect(reflection).not.toContain('apps/claw-server/package.json')
    expect(reflection).toContain('git config user.name "github-actions[bot]"')
    expect(reflection).toContain(
      'git config user.email "41898282+github-actions[bot]@users.noreply.github.com"',
    )
    expect(reflection).toContain('gh pr create')
    expect(reflection).toContain('gh pr merge "$branch" --squash --auto')
  })

  it('requires only R2 publication secrets', () => {
    for (const secret of [
      'R2_ACCOUNT_ID',
      'R2_ACCESS_KEY_ID',
      'R2_SECRET_ACCESS_KEY',
      'R2_BUCKET',
    ]) {
      expect(workflow).toContain(`${secret}:`)
    }
    expect(workflow).not.toContain('CLAW_POSTHOG')
    expect(workflow).not.toContain('SPARKLE_PRIVATE_KEY')
  })

  it('keeps onboarding separate from the sole Rust server release', () => {
    expect(browserClawWorkflow).toContain(
      'uses: ./.github/workflows/release-claw-onboard.yml',
    )
    expect(browserClawWorkflow).toContain(
      'uses: ./.github/workflows/release-claw-server-rust.yml',
    )
    expect(browserClawWorkflow).not.toContain(
      'uses: ./.github/workflows/release-claw-server.yml',
    )
    const serverJob = browserClawWorkflow.slice(
      browserClawWorkflow.indexOf('  server_browserclaw:'),
      browserClawWorkflow.indexOf('  release_linux:'),
    )
    expect(serverJob).toContain('publish_ota: false')
  })
})
