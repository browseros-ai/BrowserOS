/** The test:neo-spec wrapper: spec names, passed-through flags, and what it says when Node or the browser is missing. */
import { describe, expect, it } from 'bun:test'
import fs from 'node:fs'
import path from 'node:path'
import { findBrowser, findNode, resolveSuites, splitArgs } from './spec'

const corpus = JSON.parse(
  fs.readFileSync(path.join(import.meta.dir, 'corpus.json'), 'utf8'),
)
const suites: string[] = corpus.suites.map(
  (suite: { file: string }) => suite.file,
)

describe('resolveSuites', () => {
  it('turns short names into exactly one corpus spec each', () => {
    expect(resolveSuites(['page-goto', 'page-close'], suites)).toEqual([
      'tests/page/page-goto.spec.ts',
      'tests/library/page-close.spec.ts',
    ])
    // A bare pattern would also select page-click-react, page-click-scroll and the rest.
    expect(resolveSuites(['page-click'], suites)).toEqual([
      'tests/page/page-click.spec.ts',
    ])
  })

  it('still takes folder names, file names and full paths', () => {
    for (const name of [
      'page/page-goto',
      'page-goto.spec.ts',
      'tests/page/page-goto.spec.ts',
      '/home/me/.cache/browseros/neo-spec/playwright-1.64.1/tests/page/page-goto.spec.ts',
    ])
      expect(resolveSuites([name], suites)).toEqual([
        'tests/page/page-goto.spec.ts',
      ])
  })

  it('names close specs for an unknown name and asks for a folder when two match', () => {
    expect(() => resolveSuites(['page-got'], suites)).toThrow(
      /No spec named "page-got".*Close: page\/page-goto/,
    )
    expect(() =>
      resolveSuites(
        ['same'],
        ['tests/page/same.spec.ts', 'tests/library/same.spec.ts'],
      ),
    ).toThrow(/names 2 specs.*give its folder, e\.g\. page\/same/)
  })
})

describe('splitArgs', () => {
  it('keeps flag values with their flags and everything after -- as names', () => {
    expect(
      splitArgs([
        'page-goto',
        '--grep',
        'should work',
        '--workers=2',
        '-j',
        '1',
        '--reference',
        'page-fill',
        '--',
        'page-click',
      ]),
    ).toEqual({
      names: ['page-goto', 'page-fill', 'page-click'],
      flags: ['--grep', 'should work', '--workers=2', '-j', '1', '--reference'],
    })
  })
})

describe('findNode', () => {
  const nodes = (versions: Record<string, { node: string; bun?: string }>) => ({
    exists: (file: string) => file in versions,
    probe: (file: string) => versions[file],
  })

  it('takes the first real Node 22+ on PATH, then the Homebrew fallback', () => {
    expect(
      findNode({
        pathEnv: ['/old', '/bun', '/good'].join(path.delimiter),
        fallbacks: ['/brew'],
        ...nodes({
          '/old/node': { node: '20.11.0' },
          '/bun/node': { node: '24.3.0', bun: '1.4.2' },
          '/good/node': { node: '24.3.0' },
          '/brew/node': { node: '26.4.0' },
        }),
      }),
    ).toBe('/good/node')
    expect(
      findNode({
        pathEnv: '/old',
        fallbacks: ['/brew'],
        ...nodes({
          '/old/node': { node: '20.11.0' },
          '/brew/node': { node: '26.4.0' },
        }),
      }),
    ).toBe('/brew/node')
  })

  it('says what it rejected and how to install Node when none fits', () => {
    const message = () =>
      findNode({
        pathEnv: ['/old', '/bun'].join(path.delimiter),
        fallbacks: ['/brew'],
        ...nodes({
          '/old/node': { node: '20.11.0' },
          '/bun/node': { node: '24.3.0', bun: '1.4.2' },
        }),
      })
    expect(message).toThrow(
      /No Node\.js 22 or newer found on PATH or in \/brew/,
    )
    expect(message).toThrow(/\/old\/node \(Node 20\.11\.0\)/)
    expect(message).toThrow(/\/bun\/node \(Bun's node stand-in\)/)
    expect(message).toThrow(/brew install node@26/)
  })
})

describe('findBrowser', () => {
  const app = '/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo'

  it('uses NEO_E2E_BROWSER, else the install path for the OS', () => {
    expect(
      findBrowser({
        env: { NEO_E2E_BROWSER: '/x/neo' },
        platform: 'darwin',
        exists: () => true,
      }),
    ).toBe('/x/neo')
    expect(
      findBrowser({
        env: {},
        platform: 'darwin',
        exists: (file) => file === app,
      }),
    ).toBe(app)
  })

  it('says where it looked and what to set when the browser is missing', () => {
    const missing = { exists: () => false }
    expect(() =>
      findBrowser({ env: {}, platform: 'darwin', ...missing }),
    ).toThrow(
      /not installed at \/Applications\/BrowserOS neo\.app.*\n.*set NEO_E2E_BROWSER/,
    )
    expect(() =>
      findBrowser({
        env: { NEO_E2E_BROWSER: '/x/neo' },
        platform: 'darwin',
        ...missing,
      }),
    ).toThrow('NEO_E2E_BROWSER is /x/neo, which does not exist.')
    expect(() =>
      findBrowser({ env: {}, platform: 'win32', ...missing }),
    ).toThrow(/No default BrowserOS neo location on win32.*\n.*NEO_E2E_BROWSER/)
  })
})
