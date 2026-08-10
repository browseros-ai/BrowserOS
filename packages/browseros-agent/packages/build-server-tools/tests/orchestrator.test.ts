import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import { execFileSync } from 'node:child_process'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import {
  type ProductCompiler,
  type ResourceBuildProductDescriptor,
  runCompiledResourceBuild,
} from '../src'

describe('compiled resource orchestration', () => {
  let tempDir: string | null = null
  const originalCwd = process.cwd()
  const originalReleaseSha = process.env.RELEASE_SHA

  beforeEach(() => {
    delete process.env.RELEASE_SHA
  })

  afterEach(async () => {
    process.chdir(originalCwd)
    if (originalReleaseSha === undefined) {
      delete process.env.RELEASE_SHA
    } else {
      process.env.RELEASE_SHA = originalReleaseSha
    }
    if (tempDir) {
      await rm(tempDir, { recursive: true, force: true })
      tempDir = null
    }
  })

  it('passes resolved build inputs to a custom compiler and packages its output', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'compiled-resource-build-'))
    const rootDir = tempDir
    const packageDir = join(rootDir, 'apps/test-rust')
    const manifestPath = join(rootDir, 'resources.json')
    await mkdir(packageDir, { recursive: true })
    await writeFile(
      join(packageDir, 'Cargo.toml'),
      '[package]\nname = "test-rust"\nversion = "1.2.3"\n',
    )
    await writeFile(manifestPath, '{"resources":[]}\n')

    const product: ResourceBuildProductDescriptor = {
      label: 'Test Rust server',
      packageDir: 'apps/test-rust',
      versionSource: {
        type: 'cargo-toml',
        path: 'apps/test-rust/Cargo.toml',
      },
      distRoot: 'dist/test-rust',
      stagedBinaryBaseName: 'test-rust',
      archiveBaseName: 'test-rust-resources',
      defaultManifestPath: 'resources.json',
      defaultUpload: false,
      env: {
        requiredInlineEnvKeys: [],
        inlineEnvKeys: [],
        defaultR2UploadPrefix: 'test-rust/prod-resources',
      },
    }
    const calls: Parameters<ProductCompiler>[] = []
    const compiler: ProductCompiler = async (...args) => {
      calls.push(args)
      const [resolvedProduct, targets, , , version, options] = args
      const target = targets[0]
      if (!target) throw new Error('Expected one target')
      const binaryPath = join(rootDir, 'compiled')
      await writeFile(binaryPath, 'compiled-server')
      expect(resolvedProduct).toBe(product)
      expect(targets.map((target) => target.id)).toEqual(['darwin-arm64'])
      expect(version).toBe('1.2.3')
      expect(options).toEqual({ ci: true })
      return [{ target, binaryPath }]
    }

    await runCompiledResourceBuild(
      product,
      compiler,
      ['--target=darwin-arm64', '--ci'],
      { rootDir },
    )

    expect(calls).toHaveLength(1)
    expect(
      await readFile(
        join(rootDir, 'dist/test-rust/darwin-arm64/resources/bin/test-rust'),
        'utf8',
      ),
    ).toBe('compiled-server')
    expect(
      await Bun.file(
        join(rootDir, 'dist/test-rust/test-rust-resources-darwin-arm64.zip'),
      ).exists(),
    ).toBe(true)
  })

  it('rejects an explicit release SHA from a different checkout', async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'compiled-resource-build-'))
    const rootDir = tempDir
    const packageDir = join(rootDir, 'apps/test-rust')
    await mkdir(packageDir, { recursive: true })
    await writeFile(
      join(packageDir, 'Cargo.toml'),
      '[package]\nname = "test-rust"\nversion = "1.2.3"\n',
    )
    await writeFile(join(rootDir, 'resources.json'), '{"resources":[]}\n')
    execFileSync('git', ['init', '--quiet'], { cwd: rootDir })
    execFileSync('git', ['add', '.'], { cwd: rootDir })
    execFileSync(
      'git',
      [
        '-c',
        'user.name=BrowserOS Test',
        '-c',
        'user.email=test@browseros.com',
        'commit',
        '--quiet',
        '-m',
        'initial',
      ],
      { cwd: rootDir },
    )
    process.env.RELEASE_SHA = 'b'.repeat(40)
    const product: ResourceBuildProductDescriptor = {
      label: 'Test Rust server',
      packageDir: 'apps/test-rust',
      versionSource: {
        type: 'cargo-toml',
        path: 'apps/test-rust/Cargo.toml',
      },
      distRoot: 'dist/test-rust',
      stagedBinaryBaseName: 'test-rust',
      archiveBaseName: 'test-rust-resources',
      defaultManifestPath: 'resources.json',
      defaultUpload: false,
      includeArtifactIdentity: true,
      env: {
        requiredInlineEnvKeys: [],
        inlineEnvKeys: [],
        defaultR2UploadPrefix: 'test-rust/prod-resources',
      },
    }
    const compiler: ProductCompiler = async () => {
      throw new Error('Compiler should not run')
    }

    await expect(
      runCompiledResourceBuild(
        product,
        compiler,
        ['--target=darwin-arm64', '--ci'],
        { rootDir },
      ),
    ).rejects.toThrow('does not match checkout HEAD')
  })
})
