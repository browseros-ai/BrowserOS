/**
 * Computes which CI test suites are affected by a change, for the dynamic
 * matrix in `.github/workflows/test.yml`.
 *
 * Turbo detects affected JS + Rust workspace packages (dependents included);
 * this maps those packages, plus a couple of path signals for things that are
 * not workspace packages (root `scripts/`, `contracts/`), to the suite list
 * from `test.yml`. The suite config here is the single source of truth for
 * each suite's command, JUnit path, and browser/rust setup needs.
 *
 * The mapping (`computeAffectedSuites`) is pure and unit-tested; only `main`
 * shells out to turbo/git and writes the GitHub Actions outputs.
 */
import { execFileSync } from 'node:child_process'
import { appendFileSync } from 'node:fs'

export interface SuiteConfig {
  suite: string
  command: string
  junit_path: string
  needs_browser: boolean
  needs_rust: boolean
}

/** Every suite, in the order `test.yml` declares them (stable CI ordering). */
export const SUITES: Record<string, SuiteConfig> = {
  'server-agent': {
    suite: 'server-agent',
    command: '(cd apps/server && bun run test:agent)',
    junit_path: 'test-results/server-agent.xml',
    needs_browser: false,
    needs_rust: false,
  },
  'server-api': {
    suite: 'server-api',
    command: '(cd apps/server && bun run test:api)',
    junit_path: 'test-results/server-api.xml',
    needs_browser: true,
    needs_rust: false,
  },
  'server-tools': {
    suite: 'server-tools',
    command: '(cd apps/server && bun run test:tools)',
    junit_path: 'test-results/server-tools.xml',
    needs_browser: true,
    needs_rust: false,
  },
  'server-browser': {
    suite: 'server-browser',
    command: '(cd apps/server && bun run test:browser)',
    junit_path: 'test-results/server-browser.xml',
    needs_browser: false,
    needs_rust: false,
  },
  'server-integration': {
    suite: 'server-integration',
    command: '(cd apps/server && bun run test:integration)',
    junit_path: 'test-results/server-integration.xml',
    needs_browser: true,
    needs_rust: false,
  },
  'server-lib': {
    suite: 'server-lib',
    command: '(cd apps/server && bun run test:lib)',
    junit_path: 'test-results/server-lib.xml',
    needs_browser: false,
    needs_rust: false,
  },
  'server-root': {
    suite: 'server-root',
    command: '(cd apps/server && bun run test:root)',
    junit_path: 'test-results/server-root.xml',
    needs_browser: false,
    needs_rust: false,
  },
  agent: {
    suite: 'agent',
    command: '(cd apps/app && bun run test)',
    junit_path: 'test-results/agent.xml',
    needs_browser: false,
    needs_rust: false,
  },
  'claw-app': {
    suite: 'claw-app',
    command: '(cd apps/claw-app && bun run test)',
    junit_path: 'test-results/claw-app.xml',
    needs_browser: false,
    needs_rust: false,
  },
  'claw-onboard': {
    suite: 'claw-onboard',
    command: '(cd apps/claw-onboard && bun run test)',
    junit_path: 'test-results/claw-onboard.xml',
    needs_browser: false,
    needs_rust: false,
  },
  'app-onboard': {
    suite: 'app-onboard',
    command: '(cd apps/app-onboard && bun run test)',
    junit_path: 'test-results/app-onboard.xml',
    needs_browser: false,
    needs_rust: false,
  },
  build: {
    suite: 'build',
    command: 'bun run ./scripts/run-bun-test.ts ./scripts/build',
    junit_path: 'test-results/build.xml',
    needs_browser: false,
    needs_rust: false,
  },
  release: {
    suite: 'release',
    command: 'bun run ./scripts/run-bun-test.ts ./scripts/release',
    junit_path: 'test-results/release.xml',
    needs_browser: false,
    needs_rust: false,
  },
  'claw-server-rust': {
    suite: 'claw-server-rust',
    command: 'bun run ./scripts/run-cargo-test.ts test --workspace --locked',
    junit_path: 'test-results/claw-server-rust.xml',
    needs_browser: false,
    needs_rust: true,
  },
  'claw-server-rust-quality': {
    suite: 'claw-server-rust-quality',
    command:
      'cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings',
    junit_path: 'test-results/claw-server-rust-quality.xml',
    needs_browser: false,
    needs_rust: true,
  },
  'claw-mcp': {
    suite: 'claw-mcp',
    command: 'bun run test:claw-mcp-contract',
    junit_path: 'test-results/claw-mcp.xml',
    needs_browser: true,
    needs_rust: true,
  },
}

/** Workspace package name -> the suites that cover it. */
const PACKAGE_SUITES: Record<string, string[]> = {
  '@browseros/server': [
    'server-agent',
    'server-api',
    'server-tools',
    'server-browser',
    'server-integration',
    'server-lib',
    'server-root',
  ],
  '@browseros/app': ['agent'],
  '@browseros/claw-app': ['claw-app'],
  '@browseros/claw-onboard': ['claw-onboard'],
  '@browseros/app-onboard': ['app-onboard'],
  // The build suite exercises scripts/build, which uses build-server-tools, so
  // an affected build-server-tools (or a scripts/ change, handled below) runs it.
  '@browseros/build-server-tools': ['build'],
}

/**
 * The Rust CI suites run workspace-wide cargo, so any affected Rust crate
 * triggers all of them.
 */
const RUST_SUITES = ['claw-server-rust', 'claw-server-rust-quality', 'claw-mcp']

export interface AffectedPackage {
  name: string
  path: string
}

function isRustPackage(path: string): boolean {
  return path.startsWith('crates/') || path === 'apps/claw-server-rust'
}

const AGENT = 'packages/browseros-agent/'

/**
 * A change to the test harness itself (this workflow, this discovery script, or
 * the shared test runners used by every suite) must run the full matrix, so a
 * harness regression cannot merge without any suite exercising it.
 */
function isHarnessChange(changedFiles: string[]): boolean {
  return changedFiles.some(
    (f) =>
      f === '.github/workflows/test.yml' ||
      f.startsWith(`${AGENT}ci/`) ||
      f.startsWith(`${AGENT}scripts/run-bun-test`) ||
      f.startsWith(`${AGENT}scripts/run-cargo-test`),
  )
}

/**
 * Pure mapping from affected workspace packages + changed file paths (relative
 * to the repository root) to the suite matrix. Over-inclusive by design: bias
 * toward running a suite when uncertain, never toward skipping one.
 */
export function computeAffectedSuites(
  packages: AffectedPackage[],
  changedFiles: string[],
): SuiteConfig[] {
  if (isHarnessChange(changedFiles)) return Object.values(SUITES)

  const keys = new Set<string>()

  for (const pkg of packages) {
    for (const suite of PACKAGE_SUITES[pkg.name] ?? []) keys.add(suite)
    if (isRustPackage(pkg.path)) {
      for (const suite of RUST_SUITES) keys.add(suite)
    }
  }

  // Path signals for things that are not workspace packages.
  if (
    changedFiles.some(
      (f) =>
        f.startsWith(`${AGENT}scripts/`) &&
        !f.startsWith(`${AGENT}scripts/release/`),
    )
  )
    keys.add('build')
  if (
    changedFiles.some(
      (f) =>
        f.startsWith(`${AGENT}scripts/release/`) ||
        f === '.github/workflows/build-browseros.yml' ||
        f === '.github/workflows/nightly.yml' ||
        f === '.github/workflows/nightly-macos-product.yml' ||
        f === '.github/workflows/publish-server-ota.yml' ||
        f.startsWith('.github/workflows/release-'),
    )
  )
    keys.add('release')
  if (
    changedFiles.some(
      (f) =>
        f.startsWith(`${AGENT}contracts/claw-mcp/`) ||
        f.startsWith(`${AGENT}contracts/claw-api/`),
    )
  ) {
    keys.add('claw-mcp')
  }

  return Object.keys(SUITES)
    .filter((key) => keys.has(key))
    .map((key) => SUITES[key] as SuiteConfig)
}

function readAffectedPackages(): AffectedPackage[] {
  const raw = execFileSync(
    'bunx',
    ['turbo', 'ls', '--affected', '--output=json'],
    { encoding: 'utf8' },
  )
  const parsed = JSON.parse(raw) as {
    packages?: { items?: Array<{ name: string; path: string }> }
  }
  return (parsed.packages?.items ?? []).map((item) => ({
    name: item.name,
    path: item.path,
  }))
}

function readChangedFiles(base: string): string[] {
  const range = base ? `${base}...HEAD` : 'HEAD'
  // Repo-root-relative paths (no --relative) so `.github/**` harness changes
  // outside packages/browseros-agent are visible to isHarnessChange.
  const raw = execFileSync('git', ['diff', '--name-only', range], {
    encoding: 'utf8',
  })
  return raw.split('\n').filter(Boolean)
}

function main(): void {
  const base = process.env.BROWSEROS_AFFECTED_BASE ?? ''

  let suites: SuiteConfig[]
  let summary: string
  if (base) {
    const packages = readAffectedPackages()
    const changed = readChangedFiles(base)
    suites = computeAffectedSuites(packages, changed)
    summary = `packages=[${packages.map((p) => p.name).join(', ') || '-'}] suites=[${suites.map((s) => s.suite).join(', ') || '-'}]`
  } else {
    // No base ref (e.g. a workflow_dispatch run, which has no PR base): fail
    // safe to the full matrix so a manual run never reports green with zero
    // coverage.
    suites = Object.values(SUITES)
    summary = `no base ref, running the full matrix (${suites.length} suites)`
  }

  const matrix = { include: suites }
  const hasAny = suites.length > 0
  console.error(`[affected-suites] ${summary}`)

  // biome-ignore lint/suspicious/noUndeclaredEnvVars: GitHub injects this output path outside Turbo tasks.
  const githubOutput = process.env.GITHUB_OUTPUT
  if (githubOutput) {
    appendFileSync(githubOutput, `matrix=${JSON.stringify(matrix)}\n`)
    appendFileSync(githubOutput, `has_any=${hasAny}\n`)
    // Full suite universe so the summary comment can mark the non-affected ones.
    appendFileSync(
      githubOutput,
      `all_suites=${JSON.stringify(Object.keys(SUITES))}\n`,
    )
  }
  console.log(JSON.stringify(matrix))
}

if (import.meta.main) main()
