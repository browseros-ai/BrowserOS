import { describe, expect, it } from 'bun:test'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'

const repoRoot = resolve(import.meta.dir, '../..')
const resolver = join(repoRoot, 'scripts/release/resolve-component-release.sh')

type Component = 'agent-extension' | 'agent-server'

function packagePath(component: Component): string {
  return component === 'agent-extension'
    ? 'apps/app/package.json'
    : 'apps/server/package.json'
}

function scopedTag(component: Component, version: string): string {
  return component === 'agent-extension'
    ? `agent-extension/v${version}`
    : `agent-server/v${version}`
}

function legacyTag(component: Component, version: string): string {
  return component === 'agent-extension'
    ? `agent-extension-v${version}`
    : `browseros-server-v${version}`
}

async function run(
  cwd: string,
  args: string[],
): Promise<{ code: number; stdout: string; stderr: string }> {
  const proc = Bun.spawn(args, {
    cwd,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [stdout, stderr, code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ])
  return { code, stdout, stderr }
}

async function mustRun(cwd: string, args: string[]): Promise<string> {
  const result = await run(cwd, args)
  expect(result.code, result.stderr).toBe(0)
  return result.stdout
}

async function initFixture(
  component: Component,
  version: string,
): Promise<string> {
  const dir = mkdtempSync(join(tmpdir(), 'component-release-'))
  await mustRun(dir, ['git', 'init', '--initial-branch=main'])
  await mustRun(dir, ['git', 'config', 'user.name', 'BrowserOS Test'])
  await mustRun(dir, ['git', 'config', 'user.email', 'test@browseros.com'])
  writePackage(dir, component, version)
  await mustRun(dir, ['git', 'add', '.'])
  await mustRun(dir, ['git', 'commit', '-m', `version ${version}`])
  return dir
}

function writePackage(
  dir: string,
  component: Component,
  version: string,
): void {
  const path = join(dir, packagePath(component))
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(
    path,
    JSON.stringify(
      {
        name:
          component === 'agent-extension'
            ? '@browseros/app'
            : '@browseros/server',
        version,
      },
      null,
      2,
    ),
  )
}

async function commitVersion(
  dir: string,
  component: Component,
  version: string,
): Promise<void> {
  writePackage(dir, component, version)
  await mustRun(dir, ['git', 'add', packagePath(component)])
  await mustRun(dir, ['git', 'commit', '-m', `version ${version}`])
}

async function tag(dir: string, name: string): Promise<void> {
  await mustRun(dir, ['git', 'tag', '-a', name, '-m', name])
}

async function lightweightTag(dir: string, name: string): Promise<void> {
  await mustRun(dir, ['git', 'tag', name])
}

async function resolveRelease(dir: string, component: Component, name: string) {
  return run(dir, [
    resolver,
    '--component',
    component,
    '--tag',
    name,
    '--default-branch',
    'main',
  ])
}

function parseOutput(stdout: string): Record<string, string> {
  return Object.fromEntries(
    stdout
      .trim()
      .split('\n')
      .filter(Boolean)
      .map((line) => line.split(/=(.*)/s).slice(0, 2)),
  )
}

describe('resolve-component-release', () => {
  it('rejects non-strict slash tags', async () => {
    const dir = await initFixture('agent-extension', '0.0.100')
    try {
      for (const invalidTag of [
        'agent-extension/0.0.100',
        'agent-extension/v0.0',
        'agent-extension/v01.0.0',
        'agent-extension/v0.0.100-rc1',
        'agent-extension-v0.0.100',
      ]) {
        const result = await resolveRelease(dir, 'agent-extension', invalidTag)

        expect(result.code, invalidTag).toBe(1)
        expect(result.stderr).toContain(
          'Expected agent-extension tag like agent-extension/vX.Y.Z',
        )
      }
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('resolves an extension slash tag and previous tag across tag schemes', async () => {
    const dir = await initFixture('agent-extension', '0.0.98')
    try {
      await tag(dir, legacyTag('agent-extension', '0.0.98'))
      await commitVersion(dir, 'agent-extension', '0.0.99')
      await tag(dir, scopedTag('agent-extension', '0.0.99'))
      await commitVersion(dir, 'agent-extension', '0.0.100')
      const currentTag = scopedTag('agent-extension', '0.0.100')
      await tag(dir, currentTag)

      const result = await resolveRelease(dir, 'agent-extension', currentTag)

      expect(result.code, result.stderr).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.100',
        package_version: '0.0.100',
        tag: currentTag,
        previous_tag: scopedTag('agent-extension', '0.0.99'),
      })
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('resolves a server slash tag against legacy browseros-server tags', async () => {
    const dir = await initFixture('agent-server', '0.0.121')
    try {
      await tag(dir, legacyTag('agent-server', '0.0.121'))
      await commitVersion(dir, 'agent-server', '0.0.122')
      const currentTag = scopedTag('agent-server', '0.0.122')
      await tag(dir, currentTag)

      const result = await resolveRelease(dir, 'agent-server', currentTag)

      expect(result.code, result.stderr).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.122',
        previous_tag: legacyTag('agent-server', '0.0.121'),
      })
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('rejects lightweight component release tags', async () => {
    const dir = await initFixture('agent-server', '0.0.122')
    try {
      const currentTag = scopedTag('agent-server', '0.0.122')
      await lightweightTag(dir, currentTag)

      const result = await resolveRelease(dir, 'agent-server', currentTag)

      expect(result.code).toBe(1)
      expect(result.stderr).toContain(
        'Tag agent-server/v0.0.122 must be an annotated tag',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('rejects a tag version that does not match the package version', async () => {
    const dir = await initFixture('agent-extension', '0.0.99')
    try {
      const currentTag = scopedTag('agent-extension', '0.0.100')
      await tag(dir, currentTag)

      const result = await resolveRelease(dir, 'agent-extension', currentTag)

      expect(result.code).toBe(1)
      expect(result.stderr).toContain(
        'Tag version 0.0.100 does not match apps/app/package.json version 0.0.99',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('validates the package version from the tagged commit instead of the current checkout', async () => {
    const dir = await initFixture('agent-extension', '0.0.99')
    try {
      const currentTag = scopedTag('agent-extension', '0.0.100')
      await tag(dir, currentTag)
      await commitVersion(dir, 'agent-extension', '0.0.100')

      const result = await resolveRelease(dir, 'agent-extension', currentTag)

      expect(result.code).toBe(1)
      expect(result.stderr).toContain(
        'Tag version 0.0.100 does not match apps/app/package.json version 0.0.99',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('rejects a duplicate version from either tag scheme', async () => {
    const dir = await initFixture('agent-extension', '0.0.100')
    try {
      await tag(dir, legacyTag('agent-extension', '0.0.100'))
      const currentTag = scopedTag('agent-extension', '0.0.100')
      await tag(dir, currentTag)

      const result = await resolveRelease(dir, 'agent-extension', currentTag)

      expect(result.code).toBe(1)
      expect(result.stderr).toContain(
        'Release version 0.0.100 already exists as tag agent-extension-v0.0.100',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('rejects a non-incrementing release tag', async () => {
    const dir = await initFixture('agent-server', '0.0.101')
    try {
      await tag(dir, legacyTag('agent-server', '0.0.101'))
      await commitVersion(dir, 'agent-server', '0.0.100')
      const currentTag = scopedTag('agent-server', '0.0.100')
      await tag(dir, currentTag)

      const result = await resolveRelease(dir, 'agent-server', currentTag)

      expect(result.code).toBe(1)
      expect(result.stderr).toContain(
        'Release version 0.0.100 must be greater than latest existing agent-server version 0.0.101',
      )
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('rejects a tagged commit that is not reachable from the default branch', async () => {
    const dir = await initFixture('agent-extension', '0.0.99')
    try {
      await mustRun(dir, ['git', 'checkout', '-b', 'release-side'])
      await commitVersion(dir, 'agent-extension', '0.0.100')
      const currentTag = scopedTag('agent-extension', '0.0.100')
      await tag(dir, currentTag)

      const result = await resolveRelease(dir, 'agent-extension', currentTag)

      expect(result.code).toBe(1)
      expect(result.stderr).toContain('is not reachable from main')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })

  it('fetches the default branch ref for tag-only checkouts', async () => {
    const sourceDir = await initFixture('agent-server', '0.0.122')
    const bareDir = mkdtempSync(join(tmpdir(), 'component-release-origin-'))
    const checkoutDir = mkdtempSync(
      join(tmpdir(), 'component-release-checkout-'),
    )
    try {
      const currentTag = scopedTag('agent-server', '0.0.122')
      await tag(sourceDir, currentTag)
      await mustRun(sourceDir, ['git', 'clone', '--bare', sourceDir, bareDir])
      await mustRun(checkoutDir, ['git', 'init'])
      await mustRun(checkoutDir, ['git', 'remote', 'add', 'origin', bareDir])
      await mustRun(checkoutDir, ['git', 'fetch', 'origin', 'tag', currentTag])
      await mustRun(checkoutDir, ['git', 'checkout', currentTag])

      const result = await resolveRelease(
        checkoutDir,
        'agent-server',
        currentTag,
      )

      expect(result.code, result.stderr).toBe(0)
      expect(parseOutput(result.stdout)).toMatchObject({
        version: '0.0.122',
        tag: currentTag,
      })
    } finally {
      rmSync(sourceDir, { recursive: true, force: true })
      rmSync(bareDir, { recursive: true, force: true })
      rmSync(checkoutDir, { recursive: true, force: true })
    }
  })
})
