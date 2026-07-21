import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-extensions.yml'),
  'utf8',
)
const browserBuildWorkflow = readFileSync(
  resolve(repoRoot, '.github/workflows/build-browseros.yml'),
  'utf8',
)
const browserClawWorkflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-browserclaw.yml'),
  'utf8',
)

describe('release-extensions workflow', () => {
  it('requires the BrowserClaw PostHog key and keeps the host optional', () => {
    expect(workflow).toMatch(/VITE_CLAW_POSTHOG_KEY:\n\s+required: true/)
    expect(workflow).toMatch(/VITE_CLAW_POSTHOG_HOST:\n\s+required: false/)
    expect(workflow).toContain(
      `VITE_CLAW_POSTHOG_KEY: ${'$'}{{ secrets.VITE_CLAW_POSTHOG_KEY }}`,
    )
    expect(workflow).toContain(
      `VITE_CLAW_POSTHOG_HOST: ${'$'}{{ secrets.VITE_CLAW_POSTHOG_HOST }}`,
    )
  })

  it('forwards BrowserClaw PostHog values to local bundled builds', () => {
    expect(browserBuildWorkflow).toContain(
      `VITE_CLAW_POSTHOG_KEY: ${'$'}{{ secrets.VITE_CLAW_POSTHOG_KEY }}`,
    )
    expect(browserBuildWorkflow).toContain(
      `VITE_CLAW_POSTHOG_HOST: ${'$'}{{ secrets.VITE_CLAW_POSTHOG_HOST }}`,
    )
    expect(browserBuildWorkflow).toContain('write_env VITE_CLAW_POSTHOG_KEY')
    expect(browserBuildWorkflow).toContain('write_env VITE_CLAW_POSTHOG_HOST')
  })

  it('preflights selected BrowserClaw keys in the full release caller', () => {
    const start = browserClawWorkflow.indexOf(
      '- name: Validate selected lane configuration',
    )
    const end = browserClawWorkflow.indexOf('  server_browserclaw:', start)
    expect(start).toBeGreaterThanOrEqual(0)
    expect(end).toBeGreaterThan(start)
    const preflight = browserClawWorkflow.slice(start, end)

    expect(preflight).toContain(
      `VITE_CLAW_POSTHOG_KEY: ${'$'}{{ secrets.VITE_CLAW_POSTHOG_KEY }}`,
    )
    expect(preflight).toMatch(
      /INPUT_EXTENSIONS[\s\S]*require_value VITE_CLAW_POSTHOG_KEY/,
    )
  })
})
