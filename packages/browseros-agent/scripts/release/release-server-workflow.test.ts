import { describe, expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { resolveTargets } from '@browseros/build-server-tools'

import { rustBuildInvocation } from '../build/rust-compiler'
import {
  browserosServerBinary,
  browserosServerBuildProduct,
  browserosServerCargoEnv,
} from '../build/server/descriptor'

const repoRoot = resolve(import.meta.dir, '../../../..')
const workflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-server.yml'),
  'utf8',
)
const dollar = '$'

interface Step {
  name?: string
  uses?: string
  if?: string
  run?: string
  env?: Record<string, string>
  with?: Record<string, string | boolean>
}

interface Job {
  needs?: string | string[]
  if?: string
  name?: string
  'runs-on'?: string
  strategy?: {
    'fail-fast'?: boolean
    matrix: { include: Record<string, string>[] }
  }
  env?: Record<string, string>
  steps?: Step[]
}

const parsed = Bun.YAML.parse(workflow) as {
  env: Record<string, string>
  on: {
    workflow_call: { secrets: Record<string, { required: boolean }> }
  }
  jobs: Record<string, Job>
}

function step(jobId: string, name: string): Step {
  const found = parsed.jobs[jobId]?.steps?.find((entry) => entry.name === name)
  expect(found).toBeDefined()
  return found as Step
}

function stepIndex(jobId: string, name: string): number {
  return (parsed.jobs[jobId]?.steps ?? []).findIndex(
    (entry) => entry.name === name,
  )
}

function section(start: string, end?: string): string {
  const startIndex = workflow.indexOf(start)
  expect(startIndex).toBeGreaterThanOrEqual(0)
  const endIndex = end ? workflow.indexOf(end, startIndex + start.length) : -1
  if (end) expect(endIndex).toBeGreaterThan(startIndex)
  return workflow.slice(startIndex, endIndex === -1 ? undefined : endIndex)
}

describe('release-server workflow', () => {
  it('exposes the reusable build/finalize interface and outputs', () => {
    const call = section('  workflow_call:', '\npermissions:')
    const publishOta = call.slice(
      call.indexOf('      publish_ota:'),
      call.indexOf('    outputs:'),
    )
    expect(call).toContain('mode:')
    expect(call).toContain('default: "build"')
    expect(call).toContain('defer_finalize:')
    expect(call).toContain('default: false')
    expect(call).toContain('version:')
    expect(call).toContain('ref:')
    expect(call).toContain('publish_ota:')
    expect(publishOta).toContain('default: false')
    expect(call).toContain('outputs:')
    expect(call).toContain(`value: ${dollar}{{ jobs.prepare.outputs.version }}`)
    expect(call).toContain(`value: ${dollar}{{ jobs.prepare.outputs.tag }}`)
    expect(call).toContain(
      `value: ${dollar}{{ jobs.prepare.outputs.release_sha }}`,
    )
  })

  it('reserves a private draft without creating a public tag', () => {
    const prepare = section('  prepare:', '\n  build:\n')
    expect(prepare).toContain('browseros release component resolve')
    expect(prepare).toContain('--component server')
    expect(prepare).not.toContain('prepare-server-release.sh')
    expect(prepare).toContain('gh release create "$RELEASE_TAG"')
    expect(prepare).toContain('--draft')
    expect(prepare).toContain('--target "$RELEASE_SHA"')
    expect(prepare).not.toContain('git tag -a')
    expect(prepare).not.toContain('--draft=false')
  })

  it('checks public allocations under the component lock before mutations', () => {
    const prepare = section('  prepare:', '\n  build:\n')
    expect(workflow).toContain('group: release-server')
    expect(prepare).toContain('browseros release component resolve')
    expect(prepare.indexOf('Setup uv')).toBeLessThan(
      prepare.indexOf('Resolve release'),
    )
    expect(prepare.indexOf('Resolve release')).toBeLessThan(
      prepare.indexOf('Reserve private draft'),
    )
  })

  it('builds and attaches every immutable versioned artifact first', () => {
    const build = section('  build-publish:', '  finalize:')
    expect(browserosServerBuildProduct.env.defaultR2UploadPrefix).toBe(
      'artifacts/server',
    )
    expect(build).toContain(
      'bun scripts/build/server.ts --target=all --upload --versioned-only',
    )
    expect(build).toContain('RELEASE_SHA:')
    expect(build).toContain('Expected 5 server resource zips')
    expect(build).toContain(
      `gh release upload "$RELEASE_TAG" "${dollar}{assets[@]}" --clobber`,
    )
    expect(build).not.toContain('artifacts/server/latest/')
    expect(build).toContain(
      'browseros release component stamp --component server',
    )
    expect(build).not.toContain('package["version"] = version')
  })

  it('compiles every packaged target natively at the local glibc floor', () => {
    const build = parsed.jobs.build
    expect(build.needs).toBe('prepare')
    expect(build.if).toBe("needs.prepare.outputs.mode == 'build'")
    expect(build.name).toBe(`Build / ${dollar}{{ matrix.target }}`)
    expect(build['runs-on']).toBe(`${dollar}{{ matrix.runner }}`)
    expect(build.strategy?.['fail-fast']).toBe(false)

    const entries = new Map(
      (build.strategy?.matrix.include ?? []).map((entry) => [
        entry.target,
        entry,
      ]),
    )
    expect(
      Object.fromEntries(
        [...entries].map(([target, entry]) => [target, entry.runner]),
      ),
    ).toEqual({
      'darwin-arm64': 'macos-26',
      'darwin-x64': 'macos-26',
      'linux-arm64': 'ubuntu-24.04-arm',
      'linux-x64': 'ubuntu-latest',
      'windows-x64': 'windows-latest',
    })

    // The matrix must cover exactly the targets the packager zips, with the
    // triples and Linux glibc suffix the local cross-compiler uses.
    const targets = resolveTargets('all')
    expect([...entries.keys()].sort()).toEqual(
      targets.map((target) => target.id).sort(),
    )
    for (const target of targets) {
      const entry = entries.get(target.id) as Record<string, string>
      const local = rustBuildInvocation(target, browserosServerBinary)
      expect(entry.rust_target).toBe(local.rustTarget)
      expect(entry.cargo_target).toBe(
        local.args[local.args.indexOf('--target') + 1],
      )
      expect(entry.cargo_command).toBe(
        target.os === 'linux' ? 'zigbuild' : 'build',
      )
      expect(`browseros_server${entry.binary_ext}`).toBe(local.binaryName)
    }
    expect(entries.get('linux-x64')?.cargo_target).toBe(
      'x86_64-unknown-linux-gnu.2.17',
    )
    expect(entries.get('linux-arm64')?.cargo_target).toBe(
      'aarch64-unknown-linux-gnu.2.17',
    )
  })

  it('pins the toolchain test.yml checks with and the zig linker', () => {
    const testWorkflow = readFileSync(
      resolve(repoRoot, '.github/workflows/test.yml'),
      'utf8',
    )
    expect(parsed.env.RUST_TOOLCHAIN).toBe('1.98.0')
    expect(testWorkflow).toContain(
      `dtolnay/rust-toolchain@${parsed.env.RUST_TOOLCHAIN}`,
    )

    const rust = step('build', 'Setup Rust').run ?? ''
    expect(rust).toContain(
      'rustup toolchain install "$RUST_TOOLCHAIN" --profile minimal',
    )
    expect(rust).toContain('rustup default "$RUST_TOOLCHAIN"')
    expect(rust).toContain('rustup target add "$RUST_TARGET"')

    const zig = step('build', 'Install zig linker')
    expect(zig.if).toBe("matrix.cargo_command == 'zigbuild'")
    expect(zig.run).toContain('ziglang==0.14.1')
    expect(zig.run).toContain(
      'cargo install --locked cargo-zigbuild --version 0.23.0',
    )
  })

  it('builds with the same Cargo settings as a local server build', () => {
    for (const [name, value] of Object.entries(browserosServerCargoEnv)) {
      expect(parsed.jobs.build.env?.[name]).toBe(value)
    }
  })

  it('builds with the optional Sentry DSN and checks the stamped version', () => {
    expect(stepIndex('build', 'Stamp release version')).toBeLessThan(
      stepIndex('build', 'Build BrowserOS server'),
    )
    expect(step('build', 'Stamp release version').run).toContain(
      'browseros release component stamp --component server --version "$VERSION"',
    )

    const compile = step('build', 'Build BrowserOS server')
    expect(compile.env?.BROWSEROS_SENTRY_DSN).toBe(
      `${dollar}{{ secrets.SENTRY_DSN }}`,
    )
    expect(compile.run).toContain('::warning::SENTRY_DSN is not set')
    expect(compile.run).toContain('component_by_id("server").package_name')
    expect(compile.run).toContain(
      'cargo "$CARGO_COMMAND" --release --locked --target "$CARGO_TARGET"',
    )
    expect(compile.run).toContain('-p "$server_package"')
    expect(compile.run).toContain('--bin browseros_server')
    expect(compile.run).toContain('export MACOSX_DEPLOYMENT_TARGET=11.0')
    expect(compile.run).toContain('export MACOSX_DEPLOYMENT_TARGET=10.12')

    const verify = step('build', 'Verify stamped binary version').run ?? ''
    expect(verify).toContain(
      'binary="target/$RUST_TARGET/release/browseros_server$BINARY_EXT"',
    )
    expect(verify).toContain('actual="$("$binary" --version)"')
    expect(verify).toContain('if [ "$actual" != "$VERSION" ]')
  })

  it('hands each binary to the packaging job in the prebuilt layout', () => {
    expect(step('build', 'Stage binary for packaging').run).toContain(
      'cp "target/$RUST_TARGET/release/browseros_server$BINARY_EXT" "dist/prebuilt/$BROWSEROS_TARGET/"',
    )
    expect(step('build', 'Upload target binary')).toMatchObject({
      uses: 'actions/upload-artifact@v7',
      with: {
        name: `browseros-server-binary-${dollar}{{ matrix.target }}`,
        path: 'packages/browseros-agent/dist/prebuilt',
        'if-no-files-found': 'error',
      },
    })

    const publish = parsed.jobs['build-publish']
    expect(publish.needs).toEqual(['prepare', 'build'])
    const download = step('build-publish', 'Download target binaries')
    expect(download).toMatchObject({
      uses: 'actions/download-artifact@v8',
      with: {
        pattern: 'browseros-server-binary-*',
        'merge-multiple': true,
        path: 'packages/browseros-agent/dist/prebuilt',
      },
    })
    const pack = step(
      'build-publish',
      'Build and upload immutable server resources',
    )
    expect(pack.run).toBe(
      'bun scripts/build/server.ts --target=all --upload --versioned-only',
    )
    expect(pack.env?.BROWSEROS_SERVER_PREBUILT_DIR).toBe(
      `${dollar}{{ github.workspace }}/${download.with?.path}`,
    )
    expect(stepIndex('build-publish', 'Download target binaries')).toBeLessThan(
      stepIndex('build-publish', 'Build and upload immutable server resources'),
    )
    expect(stepIndex('build-publish', 'Setup Bun')).toBeGreaterThanOrEqual(0)
    expect(stepIndex('build-publish', 'Install Wine')).toBe(-1)
  })

  it('consumes only the secrets the Rust server build reads', () => {
    const secrets = parsed.on.workflow_call.secrets
    expect(secrets.SENTRY_DSN).toEqual({ required: false })
    expect(secrets).not.toHaveProperty('BROWSEROS_CONFIG_URL')
    expect(secrets).not.toHaveProperty('POSTHOG_API_KEY')
    expect(workflow).not.toContain('secrets.BROWSEROS_CONFIG_URL')
    expect(workflow).not.toContain('secrets.POSTHOG_API_KEY')
    expect(step('build-publish', 'Preflight required secrets').run).toContain(
      'required=(R2_ACCOUNT_ID R2_ACCESS_KEY_ID R2_SECRET_ACCESS_KEY R2_BUCKET)',
    )
  })

  it('reflects the version into the crate files the component names', () => {
    const reflect = step('reflect-version', 'Reflect version on main via PR')
      .run as string
    expect(reflect).toContain('component_by_id("server")')
    expect(reflect).toContain('spec.manifest_path.as_posix()')
    expect(reflect).toContain('spec.lockfile_path.as_posix()')
    expect(reflect).toContain('tomllib')
    expect(reflect).toContain('git add "$cargo_toml" "$lock"')
    expect(reflect).toContain(`branch="chore-bump-server-v${dollar}{VERSION}"`)
    expect(reflect).not.toContain('apps/server/package.json')
    expect(reflect).not.toContain('bun.lock')
  })

  it('finalizes only after verifying assets and versioned objects', () => {
    const finalize = section('  finalize:', '  publish-ota:')
    expect(finalize).toContain('Expected 5 draft assets')
    expect(finalize).toContain(
      `key="artifacts/server/${dollar}{VERSION}/${dollar}{name}"`,
    )
    expect(finalize).toContain('--arg component "artifacts/server"')
    expect(finalize).toContain('aws s3api get-object')
    expect(finalize).toContain('gh release download "$RELEASE_TAG"')
    expect(finalize).toContain('sha256sum')
    expect(finalize).toContain('Draft asset does not match canonical R2 object')
    expect(finalize).toContain('--metadata-directive COPY')
    const verifyIndex = finalize.indexOf('Verify prepared release')
    const tagIndex = finalize.indexOf('git tag -a "$RELEASE_TAG"')
    const publishIndex = finalize.indexOf('--draft=false')
    const latestIndex = finalize.indexOf('Copy versioned objects to latest')
    expect(verifyIndex).toBeGreaterThanOrEqual(0)
    expect(tagIndex).toBeGreaterThan(verifyIndex)
    expect(latestIndex).toBeGreaterThan(tagIndex)
    expect(publishIndex).toBeGreaterThan(latestIndex)
  })

  it('defers finalization for reusable callers and gates side effects on it', () => {
    const finalize = section('  finalize:', '  publish-ota:')
    expect(finalize).toContain("needs.prepare.outputs.mode == 'finalize'")
    expect(finalize).toContain('inputs.defer_finalize != true')
    expect(section('  publish-ota:', '  reflect-version:')).toContain(
      '- finalize',
    )
    expect(section('  reflect-version:')).toContain('- finalize')
  })

  it('merges reflection PRs through the verified release helper', () => {
    const reflection = section('  reflect-version:')
    expect(reflection).toContain('merge-release-pr.sh')
    expect(reflection).toContain('headRefOid')
    expect(reflection).not.toContain('--squash --auto')
  })

  it('publishes and persists standalone alpha releases by default', () => {
    const dispatch = section('  workflow_dispatch:', '  workflow_call:')
    const ota = section('  publish-ota:', '  reflect-version:')
    expect(dispatch).toContain('publish_ota:')
    expect(dispatch).toContain('default: true')
    expect(ota).toContain("github.event_name != 'push'")
    expect(ota).toContain('inputs.publish_ota == true')
    expect(ota).toContain('pull-requests: write')
    expect(ota).toContain('uses: ./.github/workflows/publish-server-ota.yml')
    expect(ota).toContain('product: browseros')
    expect(ota).toContain(
      `version: ${dollar}{{ needs.prepare.outputs.version }}`,
    )
    expect(ota).toContain(
      `release_sha: ${dollar}{{ needs.prepare.outputs.release_sha }}`,
    )
    expect(ota).toContain('secrets: inherit')
    expect(ota).toContain('updates/server/appcast-server.alpha.xml')
    expect(ota).not.toContain('updates/server/appcast-claw-server.alpha.xml')
  })
})
