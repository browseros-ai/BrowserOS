import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { type BuildOutput, build } from 'wxt'
import appConfig from '../../wxt.config'
import { archiveSourceMaps } from './archive-source-maps'

let fixture: string

beforeEach(async () => {
  fixture = await mkdtemp(join(tmpdir(), 'app-source-maps-'))
})

afterEach(async () => {
  await rm(fixture, { recursive: true, force: true })
})

async function write(relativePath: string, contents: string) {
  const path = join(fixture, relativePath)
  await mkdir(dirname(path), { recursive: true })
  await writeFile(path, contents)
}

function config(mode = 'production') {
  return {
    mode,
    outBaseDir: join(fixture, 'dist'),
    outDir: join(fixture, 'dist/chrome-mv3'),
  }
}

function output(): BuildOutput {
  return {
    manifest: { manifest_version: 3, name: 'Fixture', version: '1.0.0' },
    publicAssets: [],
    steps: [{ entrypoints: [], chunks: [] }],
  }
}

describe('archiveSourceMaps', () => {
  it('moves complete nested maps, refreshes only this target, and updates WXT lists', async () => {
    const map = JSON.stringify({
      version: 3,
      sources: ['main.ts'],
      sourcesContent: ['const originalName = 42'],
      mappings: 'AAAA',
      debug_id: 'test-debug-id',
    })
    await write('dist/chrome-mv3/chunks/main.js.map', map)
    await write('dist/chrome-mv3/vendor/public.js.map', 'public map')
    await write('dist/chrome-mv3/untracked.js.map', 'untracked map')
    await write('dist/chrome-mv3/chunks/main.js', 'const a=42;')
    await write('dist/chrome-mv3/icon.svg', '<svg/>')
    await write('dist/sourcemaps/chrome-mv3/stale.js.map', 'stale')
    await write('dist/sourcemaps/firefox-mv2/keep.js.map', 'other target')

    const result = output()
    result.publicAssets.push(
      { type: 'asset', fileName: 'vendor/public.js.map' },
      { type: 'asset', fileName: 'icon.svg' },
    )
    result.steps[0].chunks.push(
      { type: 'asset', fileName: 'chunks/main.js.map' },
      { type: 'chunk', fileName: 'chunks/main.js', moduleIds: [] },
    )
    const originalAssetList = result.publicAssets
    const originalChunkList = result.steps[0].chunks
    Object.freeze(result)

    await archiveSourceMaps(config(), result)

    expect(
      await readFile(
        join(fixture, 'dist/sourcemaps/chrome-mv3/chunks/main.js.map'),
        'utf8',
      ),
    ).toBe(map)
    expect(
      await readFile(
        join(fixture, 'dist/sourcemaps/chrome-mv3/vendor/public.js.map'),
        'utf8',
      ),
    ).toBe('public map')
    expect(
      await readFile(
        join(fixture, 'dist/sourcemaps/chrome-mv3/untracked.js.map'),
        'utf8',
      ),
    ).toBe('untracked map')
    expect(await readdir(config().outDir, { recursive: true })).not.toContain(
      'untracked.js.map',
    )
    expect(
      await readFile(join(config().outDir, 'chunks/main.js'), 'utf8'),
    ).toBe('const a=42;')
    expect(await readFile(join(config().outDir, 'icon.svg'), 'utf8')).toBe(
      '<svg/>',
    )
    expect(
      await readdir(join(fixture, 'dist/sourcemaps/chrome-mv3')),
    ).not.toContain('stale.js.map')
    expect(
      await readFile(
        join(fixture, 'dist/sourcemaps/firefox-mv2/keep.js.map'),
        'utf8',
      ),
    ).toBe('other target')
    expect(result.publicAssets).toBe(originalAssetList)
    expect(result.steps[0].chunks).toBe(originalChunkList)
    expect(result.publicAssets.map((file) => file.fileName)).toEqual([
      'icon.svg',
    ])
    expect(result.steps[0].chunks.map((file) => file.fileName)).toEqual([
      'chunks/main.js',
    ])
    expect(
      (await readdir(config().outDir, { recursive: true })).filter((file) =>
        file.endsWith('.map'),
      ),
    ).toEqual([])
  })

  it('leaves development output and existing archives untouched', async () => {
    await write('dist/chrome-mv3/main.js.map', 'development map')
    await write('dist/sourcemaps/chrome-mv3/keep.js.map', 'production map')
    const result = output()
    result.publicAssets.push({ type: 'asset', fileName: 'main.js.map' })

    await archiveSourceMaps(config('development'), result)

    expect(await readFile(join(config().outDir, 'main.js.map'), 'utf8')).toBe(
      'development map',
    )
    expect(
      await readFile(
        join(fixture, 'dist/sourcemaps/chrome-mv3/keep.js.map'),
        'utf8',
      ),
    ).toBe('production map')
    expect(result.publicAssets).toHaveLength(1)
  })

  it('refreshes an empty target archive when the build emits no maps', async () => {
    await write('dist/chrome-mv3/main.js', 'code')
    await write('dist/sourcemaps/chrome-mv3/stale.js.map', 'stale')
    const result = output()

    await archiveSourceMaps(config(), result)

    expect(await readdir(join(fixture, 'dist/sourcemaps/chrome-mv3'))).toEqual(
      [],
    )
    expect(await readFile(join(config().outDir, 'main.js'), 'utf8')).toBe(
      'code',
    )
    expect(result).toEqual(output())
  })

  it('propagates filesystem failures so packaging cannot continue', async () => {
    await write('dist/chrome-mv3/main.js.map', 'map')
    await write('dist/sourcemaps', 'blocks archive directory creation')
    const result = output()
    result.publicAssets.push({ type: 'asset', fileName: 'main.js.map' })

    await expect(archiveSourceMaps(config(), result)).rejects.toThrow()

    expect(await readFile(join(config().outDir, 'main.js.map'), 'utf8')).toBe(
      'map',
    )
    expect(result.publicAssets).toHaveLength(1)
  })

  it('runs the app hook after Vite upload hooks finish reading full maps', async () => {
    // Separate script entrypoints force multiple Vite builds, like the app's
    // content scripts. The recorder uses Sentry's awaited writeBundle seam.
    await write(
      'entrypoints/main.ts',
      'export default { main() { console.log("map fixture") } }',
    )
    await write(
      'entrypoints/second.ts',
      'export default { main() { console.log("second fixture") } }',
    )
    const uploadedMaps = new Map<string, string>()
    let completedUploads = 0
    let archived = false
    if (!appConfig.hooks || !('build:done' in appConfig.hooks)) {
      throw new Error('Missing app build hook')
    }
    const buildDone = appConfig.hooks['build:done']
    if (typeof buildDone !== 'function')
      throw new Error('Missing app build hook')

    const result = await build({
      root: fixture,
      configFile: false,
      mode: 'production',
      outDir: 'dist',
      modules: [],
      imports: false,
      manifest: { name: 'Source map fixture', version: '1.0.0' },
      vite: () => ({
        build: { sourcemap: 'hidden' },
        plugins: [
          {
            name: 'local-upload-recorder',
            async writeBundle(options, bundle) {
              if (!options.dir) throw new Error('Missing output directory')
              for (const fileName of Object.keys(bundle).filter((file) =>
                file.endsWith('.map'),
              )) {
                const map = await readFile(join(options.dir, fileName), 'utf8')
                expect(JSON.parse(map).sourcesContent.length).toBeGreaterThan(0)
                uploadedMaps.set(fileName, map)
              }
              completedUploads++
            },
            async closeBundle() {
              for (const [fileName, map] of uploadedMaps) {
                expect(
                  await readFile(join(config().outDir, fileName), 'utf8'),
                ).toBe(map)
              }
            },
          },
        ],
      }),
      hooks: {
        'build:done': async (wxt, buildOutput) => {
          expect(completedUploads).toBe(buildOutput.steps.length)
          expect(uploadedMaps.size).toBeGreaterThan(0)
          await buildDone(wxt, buildOutput)
          archived = true
        },
      },
    })

    expect(archived).toBe(true)
    for (const [fileName, map] of uploadedMaps) {
      expect(
        await readFile(
          join(fixture, 'dist/sourcemaps/chrome-mv3', fileName),
          'utf8',
        ),
      ).toBe(map)
    }
    for (const file of [
      ...result.publicAssets,
      ...result.steps.flatMap((step) => step.chunks),
    ]) {
      expect(file.fileName.endsWith('.map')).toBe(false)
      expect(await readFile(join(config().outDir, file.fileName))).toBeDefined()
    }
    expect(
      (await readdir(config().outDir, { recursive: true })).filter((file) =>
        file.endsWith('.map'),
      ),
    ).toEqual([])
  })
})
