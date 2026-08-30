import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-extensions.yml'),
  'utf8',
)
const feedWorkflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-extension-feeds.yml'),
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

function section(
  start: string,
  end?: string,
  source: string = workflow,
): string {
  const startIndex = source.indexOf(start)
  expect(startIndex).toBeGreaterThanOrEqual(0)
  const endIndex = end ? source.indexOf(end, startIndex + start.length) : -1
  return source.slice(startIndex, endIndex >= 0 ? endIndex : undefined)
}

describe('release-extensions workflow', () => {
  it('exposes optional allocation and build/finalize outputs', () => {
    const dispatch = section('  workflow_dispatch:', '  workflow_call:')
    expect(dispatch).toMatch(/version:[\s\S]*required: false/)
    expect(dispatch).toMatch(/mode:[\s\S]*default: "build"/)
    expect(dispatch).toMatch(/defer_finalize:[\s\S]*default: false/)
    expect(dispatch).toMatch(/publish_alpha_feed:[\s\S]*default: true/)
    expect(dispatch).toContain(
      'Explicit version; agent and BrowserOS neo allocate one when omitted',
    )

    const call = section('  workflow_call:', '\nconcurrency:')
    expect(call).toMatch(/version:[\s\S]*required: false/)
    expect(call).toMatch(/mode:[\s\S]*default: "build"/)
    expect(call).toMatch(/defer_finalize:[\s\S]*default: false/)
    expect(call).toMatch(/publish_alpha_feed:[\s\S]*default: false/)
    expect(call).toContain(
      'Explicit version; agent and BrowserOS neo allocate one when omitted',
    )
    expect(call).toContain(`value: ${'$'}{{ jobs.prepare.outputs.version }}`)
    expect(call).toContain(`value: ${'$'}{{ jobs.prepare.outputs.tag }}`)
    expect(call).toContain(
      `value: ${'$'}{{ jobs.prepare.outputs.release_sha }}`,
    )
  })

  it('keeps external and all selections on explicit versions', () => {
    const validation = section(
      '- name: Validate lifecycle inputs',
      '- name: Read allocated extension releases and tags',
    )
    expect(validation).toContain(
      '[ "$EXTENSION" != "agent" ] && [ "$EXTENSION" != "browserclaw" ]',
    )
    expect(validation).toContain('requires an explicit version')
  })

  it('reserves only private drafts before the extension build', () => {
    const prepare = section('  prepare:', '  build:')
    expect(prepare).toContain('browseros release component resolve')
    expect(prepare).toContain('--component "$EXTENSION"')
    expect(prepare).not.toContain('extension-release-records')
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

    const reflection = section('  reflect-version:')
    expect(reflection).toContain('browseros release component read')
    expect(reflection).toContain('browseros release component stamp')
    expect(reflection).toContain('merge-release-pr.sh')
    expect(reflection).toContain('headRefOid')
    expect(reflection).not.toContain('--squash --auto')
  })

  it('builds and attaches the immutable CRX before optional finalization', () => {
    const build = section('  build:', '  preflight_alpha:')
    expect(build).toContain('browseros ext release')
    expect(build).toContain(
      'bun-version-file: packages/browseros-agent/package.json',
    )
    expect(build).toContain('--source-sha "$RELEASE_SHA"')
    expect(build).toContain('gh release upload')
    expect(build).toContain('needs.prepare.outputs.version')
    expect(build).toContain('needs.prepare.outputs.release_sha')
    expect(build).toContain('browseros release component stamp')
    expect(build).toContain(
      `${'$'}{{ github.run_id }}-${'$'}{{ github.run_attempt }}`,
    )
    expect(build.indexOf('browseros ext release')).toBeLessThan(
      build.indexOf('gh release upload'),
    )
  })

  it('preflights the alpha feed before finalizing a non-deferred release', () => {
    const preflight = section('  preflight_alpha:', '  finalize:')
    expect(preflight).toContain('needs:')
    expect(preflight).toContain('- prepare')
    expect(preflight).toContain('- build')
    expect(preflight).toContain("needs.prepare.outputs.mode == 'finalize'")
    expect(preflight).toContain('inputs.defer_finalize != true')
    expect(preflight).toContain(
      `PUBLISH_ALPHA_FEED: ${'$'}{{ inputs.publish_alpha_feed }}`,
    )
    expect(preflight).not.toContain('github.event_name')
    expect(preflight).toContain('if [ "$PUBLISH_ALPHA_FEED" != "true" ]')
    expect(preflight).toContain('browseros release extensions')
    expect(preflight).toContain('args=(--channel alpha)')
    expect(preflight).toContain('args+=(--set "$NAME=$VERSION")')
    expect(preflight).toContain(
      `VERSION: ${'$'}{{ needs.prepare.outputs.version }}`,
    )
    expect(preflight).not.toContain('--publish')
    expect(preflight).toContain(
      `ref: ${'$'}{{ github.event.repository.default_branch || 'main' }}`,
    )
    expect(preflight).toContain('sha256sum "$' + '{paths[@]}" > SHA256SUMS')
    expect(preflight).toContain('uses: actions/upload-artifact@v7')
    expect(preflight).toContain(`base_sha: ${'$'}{{ steps.base.outputs.sha }}`)
    expect(preflight).toContain(
      `should_publish: ${'$'}{{ steps.render.outputs.should_publish }}`,
    )
    expect(workflow.indexOf('  preflight_alpha:')).toBeLessThan(
      workflow.indexOf('  finalize:'),
    )

    const finalize = section('  finalize:', '  publish_alpha:')
    expect(finalize).toContain('- preflight_alpha')
    expect(finalize).toContain("needs.preflight_alpha.result == 'success'")
  })

  it('finalizes without rebuilding after object and asset verification', () => {
    const finalize = section('  finalize:', '  publish_alpha:')
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

  it('commits coherent alpha snapshots before publishing those exact files', () => {
    const publish = section('  publish_alpha:')
    for (const file of [
      'updates/extensions/update-manifest.alpha.xml',
      'updates/extensions/extensions.alpha.json',
      'updates/extensions/bundled-manifest.xml',
    ]) {
      expect(publish).toContain(file)
    }
    expect(publish).not.toContain('browseros release extensions')
    expect(publish).toContain('browseros release feeds publish-local')
    expect(publish).toContain(
      `ref: ${'$'}{{ needs.preflight_alpha.outputs.base_sha }}`,
    )
    expect(publish).toContain('uses: actions/download-artifact@v7')
    expect(publish).toContain('sha256sum --check SHA256SUMS')
    expect(publish).toContain(
      "needs.preflight_alpha.outputs.should_publish == 'true'",
    )
    expect(publish).toContain('commit-update-snapshot.sh')
    expect(publish).toContain('"$DEFAULT_BRANCH"')
    expect(publish).toContain('"$' + '{paths[@]}"')
    expect(publish).not.toContain('git push origin "HEAD:$DEFAULT_BRANCH"')
    expect(publish).not.toContain('--force')
    expect(publish.indexOf('commit-update-snapshot.sh')).toBeLessThan(
      publish.indexOf('browseros release feeds publish-local'),
    )

    const feedArtifact = section(
      '- name: Upload exact alpha feed snapshot',
      '  finalize:',
    )
    expect(feedArtifact).not.toContain('R2_SECRET_ACCESS_KEY')
    expect(feedArtifact).not.toContain('BROWSERCLAW_KEY')
  })

  it('serializes releases and manual feed publication in one concurrency group', () => {
    expect(workflow).toMatch(
      /concurrency:\n\s+group: release-extensions-and-feeds\n\s+cancel-in-progress: false/,
    )
    expect(feedWorkflow).toMatch(
      /concurrency:\n\s+group: release-extensions-and-feeds\n\s+cancel-in-progress: false/,
    )
    expect(section('on:', '\npermissions:')).not.toMatch(/\n {2}push:/)
  })

  it('persists manual feed snapshots before publishing their exact files', () => {
    const job = section('  feeds:', undefined, feedWorkflow)
    const transaction = section(
      '- name: Generate, persist, and publish extension update feeds',
      undefined,
      feedWorkflow,
    )
    const channelLoopStart = transaction.indexOf(
      `for feed_channel in "\${channels[@]}"`,
    )
    expect(channelLoopStart).toBeGreaterThanOrEqual(0)
    const channelLoop = transaction.slice(channelLoopStart)
    const renderCommand = [
      'uv run --directory packages/browseros browseros \\',
      `              release extensions "\${args[@]}"`,
    ].join('\n')
    const commitCommand = [
      'packages/browseros-agent/scripts/release/commit-update-snapshot.sh \\',
      '              "$DEFAULT_BRANCH" \\',
      `              "chore(release): update extension \${feed_channel} feeds" \\`,
      `              "\${snapshot_paths[@]}"`,
    ].join('\n')
    const publishCommand = [
      'uv run --directory packages/browseros browseros \\',
      '              release feeds publish-local \\',
      `              "\${feed_keys[@]}" \\`,
      `              "\${publish_flags[@]}"`,
    ].join('\n')
    const channelLoopEnd = channelLoop.indexOf(
      '\n          done\n\n          if [ "$PUBLISH"',
    )

    expect(job).toMatch(
      /permissions:\n\s+contents: write\n\s+pull-requests: write/,
    )
    expect(job).toMatch(/uses: actions\/checkout@[0-9a-f]{40} # v7/)
    expect(job).toMatch(/uses: astral-sh\/setup-uv@[0-9a-f]{40} # v8\.3\.2/)
    expect(job).toContain('timeout-minutes: 40')
    expect(job).toContain('fetch-depth: 0')
    expect(job).toContain(
      `ref: ${'$'}{{ github.event.repository.default_branch || 'main' }}`,
    )
    expect(transaction).toContain('both) channels=(alpha prod)')
    expect(transaction).toContain('extensions/update-manifest.alpha.xml')
    expect(transaction).toContain('extensions/extensions.alpha.json')
    expect(transaction).toContain('extensions/update-manifest.xml')
    expect(transaction).toContain('extensions/extensions.json')
    expect(transaction).toContain('extensions/bundled-manifest.xml')
    expect(
      transaction.match(/extensions\/bundled-manifest\.xml/g),
    ).toHaveLength(2)
    expect(transaction).toContain('snapshot_paths+=("updates/$feed_key")')
    expect(transaction).not.toContain('updates/extensions/update-manifest')
    expect(transaction).toContain('if [ "$PUBLISH" != "true" ]')
    expect(transaction).toContain('continue')
    expect(transaction).toContain('commit-update-snapshot.sh')
    expect(transaction).toContain('release feeds publish-local')
    expect(transaction).toContain('--publish')
    expect(transaction).not.toContain('base_args+=(--publish)')
    expect(transaction).not.toContain('args+=(--publish)')
    expect(transaction).toContain('base_args+=(--allow-downgrade)')
    expect(transaction).toContain('publish_flags=(--publish)')
    expect(transaction).toContain('publish_flags+=(--allow-downgrade)')
    expect(channelLoop.indexOf(renderCommand)).toBeGreaterThanOrEqual(0)
    expect(channelLoop.indexOf(commitCommand)).toBeGreaterThanOrEqual(0)
    expect(channelLoop.indexOf(publishCommand)).toBeGreaterThanOrEqual(0)
    expect(channelLoopEnd).toBeGreaterThanOrEqual(0)
    expect(channelLoop.indexOf(renderCommand)).toBeLessThan(
      channelLoop.indexOf(commitCommand),
    )
    expect(channelLoop.indexOf(commitCommand)).toBeLessThan(
      channelLoop.indexOf(publishCommand),
    )
    expect(channelLoop.indexOf(publishCommand)).toBeLessThan(channelLoopEnd)
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

  it('forwards BrowserClaw extension secrets to its dedicated release', () => {
    const start = browserClawWorkflow.indexOf('  extension:')
    const end = browserClawWorkflow.indexOf('  components:', start)
    expect(start).toBeGreaterThanOrEqual(0)
    expect(end).toBeGreaterThan(start)
    const extension = browserClawWorkflow.slice(start, end)

    expect(extension).toContain(
      'uses: ./.github/workflows/release-extensions.yml',
    )
    expect(extension).toContain('extension: browserclaw')
    expect(extension).toContain('secrets: inherit')
    expect(browserClawWorkflow).not.toContain(
      'browseros release resources prepare',
    )
  })

  it('keeps extension build secrets out of native browser lanes', () => {
    expect(browserBuildWorkflow).not.toContain(
      `VITE_CLAW_POSTHOG_KEY: ${'$'}{{ secrets.VITE_CLAW_POSTHOG_KEY }}`,
    )
    expect(browserBuildWorkflow).not.toContain(
      `VITE_CLAW_POSTHOG_HOST: ${'$'}{{ secrets.VITE_CLAW_POSTHOG_HOST }}`,
    )
    expect(browserBuildWorkflow).not.toContain(
      `BROWSERCLAW_KEY: ${'$'}{{ secrets.BROWSERCLAW_KEY }}`,
    )
  })
})
