import { describe, expect, it } from 'bun:test'
import {
  type AffectedPackage,
  computeAffectedSuites,
  SUITES,
} from './affected-suites'

function pkg(name: string, path: string): AffectedPackage {
  return { name, path }
}

function suiteNames(
  packages: AffectedPackage[],
  changedFiles: string[] = [],
): string[] {
  return computeAffectedSuites(packages, changedFiles).map((s) => s.suite)
}

describe('computeAffectedSuites', () => {
  it('returns nothing when nothing is affected', () => {
    expect(suiteNames([], [])).toEqual([])
  })

  it('maps a claw-app change to only the claw-app suite', () => {
    expect(suiteNames([pkg('@browseros/claw-app', 'apps/claw-app')])).toEqual([
      'claw-app',
    ])
  })

  it('maps an app-onboard change to only the app-onboard suite', () => {
    expect(
      suiteNames([pkg('@browseros/app-onboard', 'apps/app-onboard')]),
    ).toEqual(['app-onboard'])
  })

  it('maps the server package to all seven server suites', () => {
    expect(suiteNames([pkg('@browseros/server', 'apps/server')])).toEqual([
      'server-agent',
      'server-api',
      'server-tools',
      'server-browser',
      'server-integration',
      'server-lib',
      'server-root',
    ])
  })

  it('maps the app package to the agent suite', () => {
    expect(suiteNames([pkg('@browseros/app', 'apps/app')])).toEqual(['agent'])
  })

  it('maps any affected Rust crate to all three Rust suites', () => {
    expect(
      suiteNames([
        pkg('@browseros/browseros-core-rust', 'crates/browseros-core'),
      ]),
    ).toEqual(['claw-server-rust', 'claw-server-rust-quality', 'claw-mcp'])
  })

  it('treats the Rust server app path as a Rust package', () => {
    expect(
      suiteNames([pkg('@browseros/claw-server-rust', 'apps/claw-server-rust')]),
    ).toEqual(['claw-server-rust', 'claw-server-rust-quality', 'claw-mcp'])
  })

  it('adds the build suite when scripts change', () => {
    expect(
      suiteNames([], ['packages/browseros-agent/scripts/build/server.ts']),
    ).toEqual(['build'])
  })

  it('adds the release suite for release scripts and workflows', () => {
    expect(
      suiteNames(
        [],
        ['packages/browseros-agent/scripts/release/commit-update-snapshot.sh'],
      ),
    ).toEqual(['release'])
    for (const familyWorkflow of [
      '.github/workflows/nightly.yml',
      '.github/workflows/nightly-macos-product.yml',
    ]) {
      expect(suiteNames([], [familyWorkflow])).toEqual(['release'])
    }
  })

  it('runs the full matrix on a test-harness change', () => {
    for (const harnessFile of [
      '.github/workflows/test.yml',
      'packages/browseros-agent/ci/affected-suites.ts',
      'packages/browseros-agent/scripts/run-bun-test.ts',
      'packages/browseros-agent/scripts/run-cargo-test.ts',
    ]) {
      expect(new Set(suiteNames([], [harnessFile]))).toEqual(
        new Set(Object.keys(SUITES)),
      )
    }
  })

  it('adds the build suite when build-server-tools is affected', () => {
    expect(
      suiteNames([
        pkg('@browseros/build-server-tools', 'packages/build-server-tools'),
      ]),
    ).toEqual(['build'])
  })

  it('adds claw-mcp when the claw-api or claw-mcp contract changes', () => {
    expect(
      suiteNames(
        [],
        ['packages/browseros-agent/contracts/claw-api/openapi.yaml'],
      ),
    ).toEqual(['claw-mcp'])
    expect(
      suiteNames(
        [],
        ['packages/browseros-agent/contracts/claw-mcp/tools.json'],
      ),
    ).toEqual(['claw-mcp'])
  })

  it('ignores unrelated non-package files', () => {
    expect(suiteNames([], ['README.md', 'docs/whatever.md'])).toEqual([])
  })

  it('unions and de-duplicates across packages and paths, in declared order', () => {
    const result = suiteNames(
      [
        pkg('@browseros/claw-app', 'apps/claw-app'),
        pkg('@browseros/app', 'apps/app'),
        pkg('@browseros/browseros-mcp-rust', 'crates/browseros-mcp'),
      ],
      ['packages/browseros-agent/contracts/claw-api/openapi.yaml'],
    )
    // claw-mcp appears once even though both the rust crate and the contract
    // path request it, and the order follows the SUITES declaration.
    expect(result).toEqual([
      'agent',
      'claw-app',
      'claw-server-rust',
      'claw-server-rust-quality',
      'claw-mcp',
    ])
  })

  it('surfaces every package-owned suite when a universal dependency changes', () => {
    // A change to @browseros/shared marks all packages affected (Turbo global
    // hash); the mapping should then request every suite.
    const allPackages: AffectedPackage[] = [
      pkg('@browseros/server', 'apps/server'),
      pkg('@browseros/app', 'apps/app'),
      pkg('@browseros/claw-app', 'apps/claw-app'),
      pkg('@browseros/claw-onboard', 'apps/claw-onboard'),
      pkg('@browseros/app-onboard', 'apps/app-onboard'),
      pkg('@browseros/build-server-tools', 'packages/build-server-tools'),
      pkg('@browseros/claw-server-rust', 'apps/claw-server-rust'),
    ]
    expect(new Set(suiteNames(allPackages))).toEqual(
      new Set(Object.keys(SUITES).filter((suite) => suite !== 'release')),
    )
  })

  it('emits full suite config, not just names', () => {
    const [claw] = computeAffectedSuites(
      [pkg('@browseros/claw-app', 'apps/claw-app')],
      [],
    )
    expect(claw).toEqual({
      suite: 'claw-app',
      command: '(cd apps/claw-app && bun run test)',
      junit_path: 'test-results/claw-app.xml',
      needs_browser: false,
      needs_rust: false,
    })
  })
})
