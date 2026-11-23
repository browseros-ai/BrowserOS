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

  it('maps a neo-viewer change to only the neo-viewer suite', () => {
    expect(
      computeAffectedSuites(
        [pkg('@browseros/neo-viewer', 'tools/neo-viewer')],
        [],
      ),
    ).toEqual([
      {
        suite: 'neo-viewer',
        command:
          'bun run ./scripts/run-bun-test.ts --cwd=./tools/neo-viewer ./tools/neo-viewer/server ./tools/neo-viewer/shared ./tools/neo-viewer/web',
        junit_path: 'test-results/neo-viewer.xml',
        needs_browser: false,
        needs_rust: false,
      },
    ])
  })

  it('maps the app package to the agent suite', () => {
    expect(suiteNames([pkg('@browseros/app', 'apps/app')])).toEqual(['agent'])
  })

  it('tests that the cursor package selects its own and pointer integration suites', () => {
    const suites = computeAffectedSuites(
      [pkg('@browseros/agent-cursor', 'packages/agent-cursor')],
      [],
    )
    expect(suites).toEqual([
      {
        suite: 'agent-cursor',
        command: '(cd packages/agent-cursor && bun run test)',
        junit_path: 'test-results/agent-cursor.xml',
        needs_browser: false,
        needs_rust: false,
      },
      {
        suite: 'build',
        command: 'bun run ./scripts/run-bun-test.ts ./scripts/build',
        junit_path: 'test-results/build.xml',
        needs_browser: false,
        needs_rust: false,
      },
    ])
  })

  it('tests that a committed cursor asset change selects its freshness suite', () => {
    expect(
      suiteNames(
        [],
        [
          'packages/browseros-agent/crates/browseros-core/src/assets/agent-cursor.js',
        ],
      ),
    ).toEqual(['agent-cursor', 'build', 'sdk-desktop-fixtures'])
  })

  it('tests that a vendored Playwright or engine asset change selects the engine freshness suite', () => {
    for (const changed of [
      'packages/browseros-agent/crates/neo-sdk/neo-playwright/assets/injected.js',
      'packages/browseros-agent/crates/neo-sdk/neo-playwright/assets/prelude.js',
      'packages/browseros-agent/crates/neo-sdk/neo-playwright/tests/data/probe-protocol.json',
      'packages/browseros-agent/crates/neo-sdk/neo-playwright/third_party/playwright/packages/injected/src/injectedScript.ts',
    ]) {
      expect(suiteNames([], [changed])).toEqual([
        'browseros-playwright-js',
        'sdk-desktop-fixtures',
      ])
    }
  })

  it('maps any affected Rust crate to all Rust suites', () => {
    expect(
      suiteNames([pkg('@browseros/unrelated-rust', 'crates/unrelated')]),
    ).toEqual(['claw-server', 'claw-server-quality', 'claw-mcp'])
  })

  it('treats the Rust server app path as a Rust package', () => {
    expect(
      suiteNames([pkg('@browseros/claw-server', 'apps/claw-server')]),
    ).toEqual(['claw-server', 'claw-server-quality', 'claw-mcp'])
  })

  it('does not select SDK fixtures for a claw-server-only change', () => {
    expect(
      suiteNames(
        [pkg('@browseros/claw-server', 'apps/claw-server')],
        ['packages/browseros-agent/apps/claw-server/src/lib.rs'],
      ),
    ).not.toContain('sdk-desktop-fixtures')
  })

  it('selects SDK fixtures for a browseros-core change', () => {
    expect(
      suiteNames(
        [pkg('@browseros/browseros-core-rust', 'crates/browseros-core')],
        ['packages/browseros-agent/crates/browseros-core/src/lib.rs'],
      ),
    ).toEqual([
      'claw-server',
      'claw-server-quality',
      'claw-mcp',
      'sdk-desktop-fixtures',
    ])
  })

  it('selects SDK fixtures from each watched package root without changed-file metadata', () => {
    for (const path of [
      'crates/neo-sdk/neo-captcha',
      'crates/neo-sdk/neo-playwright-js',
      'crates/browseros-core',
      'crates/browseros-cdp',
      'crates/browseros-mcp',
    ]) {
      expect(suiteNames([pkg('test-package', path)])).toContain(
        'sdk-desktop-fixtures',
      )
    }
  })

  it('selects SDK fixtures from each watched path without affected-package metadata', () => {
    for (const path of [
      'crates/neo-sdk/neo-captcha/Cargo.toml',
      'crates/neo-sdk/neo-server/tests/desktop-fixtures/README.md',
      'crates/browseros-core/src/lib.rs',
      'crates/browseros-cdp/src/lib.rs',
      'crates/browseros-mcp/src/lib.rs',
      'contracts/claw-mcp/fixtures/pages/act.html',
      'contracts/claw-mcp/tests/cases-act.ts',
    ]) {
      expect(suiteNames([], [`packages/browseros-agent/${path}`])).toContain(
        'sdk-desktop-fixtures',
      )
    }
  })

  it('keeps unrelated and similarly named Rust paths out of SDK fixtures', () => {
    for (const path of [
      'apps/claw-server',
      'crates/unrelated',
      'crates/neo-sdk-other',
      'crates/browseros-core-other',
    ]) {
      expect(
        suiteNames(
          [pkg('test-package', path)],
          [`packages/browseros-agent/${path}/src/lib.rs`],
        ),
      ).not.toContain('sdk-desktop-fixtures')
    }
  })

  it('runs the Rust suites and its own check script for a claw-sdk change', () => {
    expect(
      suiteNames([pkg('@browseros/neo-server', 'crates/neo-sdk/neo-server')]),
    ).toEqual([
      'claw-server',
      'claw-server-quality',
      'claw-sdk-quality',
      'claw-mcp',
      'sdk-desktop-fixtures',
    ])
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
    for (const nightlyWorkflow of [
      '.github/workflows/nightly-macos-browseros.yml',
      '.github/workflows/nightly-macos-browserclaw.yml',
      '.github/workflows/reusable-build-macos-nightly.yml',
    ]) {
      expect(suiteNames([], [nightlyWorkflow])).toEqual(['release'])
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
    ).toEqual(['claw-mcp', 'sdk-desktop-fixtures'])
  })

  it('ignores unrelated non-package files', () => {
    expect(suiteNames([], ['README.md', 'docs/whatever.md'])).toEqual([])
  })

  it('tests that CAPTCHA test bed changes select SDK fixtures and the existing unit suite', () => {
    const suites = computeAffectedSuites(
      [],
      [
        'packages/browseros-agent/crates/neo-sdk/neo-captcha/testbed/e2e/runner.ts',
      ],
    )
    expect(suites.map((suite) => suite.suite)).toEqual([
      'sdk-desktop-fixtures',
      'captcha',
    ])
    expect(suites.find((suite) => suite.suite === 'captcha')).toMatchObject({
      command:
        'bun run ./scripts/run-bun-test.ts ./crates/neo-sdk/neo-captcha/testbed',
      junit_path: 'test-results/captcha.xml',
      needs_browser: false,
      needs_rust: false,
    })
  })

  it('tests that a change to the shipped behaviour file selects the claw-server suite', () => {
    expect(suiteNames([], ['packages/browseros/docker/neo/neo.toml'])).toEqual([
      'claw-server',
    ])
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
      'claw-server',
      'claw-server-quality',
      'claw-mcp',
      'sdk-desktop-fixtures',
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
      pkg('@browseros/claw-server', 'apps/claw-server'),
      pkg('@browseros/neo-server', 'crates/neo-sdk/neo-server'),
      pkg('@browseros/agent-cursor', 'packages/agent-cursor'),
      pkg('@browseros/neo-playwright-js', 'crates/neo-sdk/neo-playwright-js'),
      pkg('@browseros/neo-viewer', 'tools/neo-viewer'),
    ]
    expect(new Set(suiteNames(allPackages))).toEqual(
      new Set(
        Object.keys(SUITES).filter(
          (suite) => !['release', 'captcha'].includes(suite),
        ),
      ),
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
