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

function section(start: string, end?: string): string {
  const startIndex = workflow.indexOf(start)
  expect(startIndex).toBeGreaterThanOrEqual(0)
  const endIndex = end ? workflow.indexOf(end, startIndex + start.length) : -1
  return workflow.slice(startIndex, endIndex >= 0 ? endIndex : undefined)
}

describe('release-extensions workflow', () => {
  it('exposes optional allocation and build/finalize outputs', () => {
    const dispatch = section('  workflow_dispatch:', '  workflow_call:')
    expect(dispatch).toMatch(/version:[\s\S]*required: false/)
    expect(dispatch).toMatch(/mode:[\s\S]*default: "build"/)
    expect(dispatch).toMatch(/defer_finalize:[\s\S]*default: false/)

    const call = section('  workflow_call:', '\nconcurrency:')
    expect(call).toMatch(/version:[\s\S]*required: false/)
    expect(call).toMatch(/mode:[\s\S]*default: "build"/)
    expect(call).toMatch(/defer_finalize:[\s\S]*default: false/)
    expect(call).toContain(`value: ${'$'}{{ jobs.prepare.outputs.version }}`)
    expect(call).toContain(`value: ${'$'}{{ jobs.prepare.outputs.tag }}`)
    expect(call).toContain(
      `value: ${'$'}{{ jobs.prepare.outputs.release_sha }}`,
    )
  })

  it('reserves only private drafts before the extension build', () => {
    const prepare = section('  prepare:', '  build:')
    expect(prepare).toContain('resolve-version')
    expect(prepare).toContain('tag_object: $object')
    expect(prepare).toContain('tag_object: ($tag.tag_object // "missing")')
    expect(prepare).toContain('--draft')
    expect(prepare).not.toContain('git tag -a')
    expect(prepare).not.toContain('--draft=false')
    expect(workflow.indexOf('  prepare:')).toBeLessThan(
      workflow.indexOf('  build:'),
    )
  })

  it('reuses drafts and published releases as distinct lifecycle states', () => {
    const prepare = section('  prepare:', '  build:')
    expect(prepare).toContain('Reused private draft')
    expect(prepare).toContain('Reused already-published release')
    expect(prepare).toContain(
      'Published release $TAG requires an annotated tag at $RELEASE_SHA; historical lightweight tags need a new version',
    )
    expect(prepare).toContain('git cat-file -t "refs/tags/$TAG"')

    const publish = section('- name: Publish private drafts')
    expect(publish).toContain("--json isDraft --jq '.isDraft'")
    expect(publish).toContain('Release $TAG is already published')
  })

  it('builds and attaches the immutable CRX before optional finalization', () => {
    const build = section('  build:', '  finalize:')
    expect(build).toContain('browseros ext release')
    expect(build).toContain('--source-sha "$RELEASE_SHA"')
    expect(build).toContain('gh release upload')
    expect(build).toContain('needs.prepare.outputs.version')
    expect(build).toContain('needs.prepare.outputs.release_sha')
    expect(build).toContain(
      `${'$'}{{ github.run_id }}-${'$'}{{ github.run_attempt }}`,
    )
    expect(build.indexOf('browseros ext release')).toBeLessThan(
      build.indexOf('gh release upload'),
    )
  })

  it('finalizes without rebuilding after object and asset verification', () => {
    const finalize = section('  finalize:')
    expect(finalize).toContain('browseros ext verify')
    expect(finalize).toContain('--source-sha "$RELEASE_SHA"')
    expect(finalize).toContain('--output-dir "$canonical_dir"')
    expect(finalize).not.toContain('browseros ext release')
    expect(finalize).toContain("'([.assets[].name] | sort) == [$asset]'")
    expect(finalize).toContain('gh release download')
    expect(finalize).toContain(
      'cmp --silent "$canonical_dir/$asset" "$draft_dir/$asset"',
    )
    expect(finalize).toContain('git cat-file -t "refs/tags/$TAG"')
    const verifyIndex = finalize.indexOf('Verify prepared extension release')
    const tagIndex = finalize.indexOf('git tag -a "$TAG"')
    const publishIndex = finalize.indexOf('--draft=false')
    expect(verifyIndex).toBeGreaterThanOrEqual(0)
    expect(tagIndex).toBeGreaterThan(verifyIndex)
    expect(publishIndex).toBeGreaterThan(tagIndex)
  })

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
    const end = browserClawWorkflow.indexOf('  build_onboarding:', start)
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
