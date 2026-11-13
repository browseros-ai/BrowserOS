/** The test:neo-spec wrapper: spec names, passed-through flags, and what it says when Node or the browser is missing. */
import { afterEach, describe, expect, it, spyOn } from 'bun:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  ciArgs,
  ciRerunArgs,
  ensureSource,
  exactGrep,
  findBrowser,
  findNode,
  finish,
  isFiltered,
  publishSource,
  rerunFlags,
  rerunLine,
  resolveSuites,
  runnerEnv,
  sourceDir,
  splitArgs,
  sweepStaging,
} from './spec'

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

describe('flags', () => {
  it('passes every option that takes a value through with its value', () => {
    expect(
      splitArgs([
        '--global-timeout',
        '60000',
        '--project',
        'browseros-neo',
        '--engine',
        'reference',
        'page-goto',
      ]),
    ).toEqual({
      names: ['page-goto'],
      flags: [
        '--global-timeout',
        '60000',
        '--project',
        'browseros-neo',
        '--engine',
        'reference',
      ],
    })
  })

  it('keeps how a run ran and drops what chose its tests, for a one-test rerun', () => {
    expect(
      rerunFlags([
        '--reference',
        '--no-curation',
        '--hosted-mode',
        '--grep',
        'should work',
        '-G=other',
        '--shard',
        '1/2',
        '--workers',
        '2',
        '--last-failed',
      ]),
    ).toEqual([
      '--reference',
      '--no-curation',
      '--hosted-mode',
      '--workers',
      '2',
    ])
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

const scratch: string[] = []
const tempDir = () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-spec-test-'))
  scratch.push(dir)
  return dir
}
afterEach(() => {
  for (const dir of scratch.splice(0))
    fs.rmSync(dir, { recursive: true, force: true })
})

describe('preparing the source', () => {
  const build = (marker: string) => (staging: string) => {
    fs.mkdirSync(staging)
    fs.writeFileSync(path.join(staging, '.neo-source.json'), marker)
  }

  it('two first runs never delete what the other prepared or published', () => {
    const source = path.join(tempDir(), 'playwright-1.64.1')
    // Run B starts first; while it builds, run A prepares and publishes.
    publishSource(
      source,
      (staging) => {
        build('B')(staging)
        publishSource(source, build('A'), 1)
        expect(
          fs.readFileSync(path.join(staging, '.neo-source.json'), 'utf8'),
        ).toBe('B')
      },
      2,
    )
    expect(fs.readFileSync(path.join(source, '.neo-source.json'), 'utf8')).toBe(
      'A',
    )
    expect(fs.readdirSync(path.dirname(source))).toEqual(['playwright-1.64.1'])
  })

  it('leaves nothing behind when preparation fails', () => {
    const source = path.join(tempDir(), 'playwright-1.64.1')
    expect(() =>
      publishSource(source, (staging) => {
        fs.mkdirSync(staging)
        throw new Error('npm ci failed')
      }),
    ).toThrow('npm ci failed')
    expect(fs.readdirSync(path.dirname(source))).toEqual([])
  })

  it('sweeps only what runs that died left behind', () => {
    const cache = tempDir()
    const source = path.join(cache, 'playwright-1.64.1')
    const entries = [
      'playwright-1.64.1',
      'playwright-1.64.1.staging-11',
      'playwright-1.64.1.staging-11.checkout',
      'playwright-1.64.1.staging-11.log',
      'playwright-1.64.1.staging-22',
      'playwright-1.65.0.staging-11',
    ]
    for (const entry of entries) fs.mkdirSync(path.join(cache, entry))
    sweepStaging(source, (pid) => pid === 22)
    expect(fs.readdirSync(cache).sort()).toEqual([
      'playwright-1.64.1',
      'playwright-1.64.1.staging-22',
      'playwright-1.65.0.staging-11',
    ])
  })

  const stamp = (source: string) =>
    fs.readFileSync(path.join(source, '.neo-source.json'), 'utf8')
  const unused = () => {
    throw new Error('must not prepare again')
  }

  it('uses a cache another run publishes while this run checks it', () => {
    const env = { NEO_SPEC_CACHE_DIR: tempDir() }
    const source = sourceDir(env)
    const existsSync = fs.existsSync
    let published = false
    // Run A's rename lands right after this run's first look at the folder.
    const spy = spyOn(fs, 'existsSync').mockImplementation((file) => {
      const seen = existsSync(file)
      if (!published && String(file).startsWith(source)) {
        published = true
        build('A')(source)
      }
      return seen
    })
    try {
      expect(ensureSource('/n/node', env, { build: build('B') })).toBe(source)
    } finally {
      spy.mockRestore()
    }
    expect(stamp(source)).toBe('A')
  })

  it('sweeps what dead runs left even when the cache is ready', () => {
    const cache = tempDir()
    const env = { NEO_SPEC_CACHE_DIR: cache }
    build('A')(sourceDir(env))
    fs.mkdirSync(path.join(cache, 'playwright-1.64.1.staging-11'))
    fs.mkdirSync(path.join(cache, 'playwright-1.64.1.staging-11.checkout'))
    ensureSource('/n/node', env, { build: unused, isAlive: () => false })
    expect(fs.readdirSync(cache)).toEqual(['playwright-1.64.1'])
  })

  it('clears what an earlier run with the same pid left before building', () => {
    const cache = tempDir()
    const source = path.join(cache, 'playwright-1.64.1')
    // A killed run left its clone; git clone refuses an existing destination.
    fs.mkdirSync(`${source}.staging-7.checkout`)
    fs.writeFileSync(`${source}.staging-7.log`, 'old')
    publishSource(
      source,
      (staging) => {
        expect(fs.existsSync(`${staging}.checkout`)).toBe(false)
        expect(fs.existsSync(`${staging}.log`)).toBe(false)
        build('B')(staging)
      },
      7,
    )
    expect(fs.readdirSync(cache)).toEqual(['playwright-1.64.1'])
  })

  it('resolves a relative cache folder, since git and npm run elsewhere', () => {
    expect(sourceDir({ NEO_SPEC_CACHE_DIR: 'neo-cache' })).toBe(
      path.resolve('neo-cache', 'playwright-1.64.1'),
    )
  })
})

describe('--list', () => {
  it('needs no browser and is not a failed run', async () => {
    const missing = () => {
      throw new Error('BrowserOS neo is not installed')
    }
    expect(
      runnerEnv({
        node: '/n/node',
        flags: ['--list'],
        env: {},
        browser: missing,
      }).PATH,
    ).toStartWith('/n')
    const output = tempDir()
    const errors: string[] = []
    const code = await finish(0, output, ['--list'], {
      log: () => {},
      error: (text) => errors.push(text),
    })
    expect({ code, errors }).toEqual({ code: 0, errors: [] })
  })
})

describe('the closing rerun line', () => {
  const spec = (
    title: string,
    line: number,
    status: string,
    results: string[],
  ) => ({
    title,
    line,
    column: 3,
    tests: [{ status, results: results.map((result) => ({ status: result })) }],
  })
  const runDir = (signal: string | null) => {
    const output = tempDir()
    const file = 'page/page-click.spec.ts'
    const results = {
      suites: [
        {
          title: file,
          file,
          specs: [
            spec('should click the button @smoke', 24, 'expected', ['passed']),
            spec(
              'should click on a span with an inline element inside',
              75,
              'skipped',
              ['interrupted'],
            ),
            spec(
              'should not throw UnhandledPromiseRejection when page closes',
              99,
              'skipped',
              [],
            ),
          ],
        },
      ],
    }
    fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify(results))
    fs.writeFileSync(path.join(output, 'run.json'), JSON.stringify({ signal }))
    fs.copyFileSync(
      path.join(import.meta.dir, 'node/scope.json'),
      path.join(output, 'scope.json'),
    )
    return output
  }

  it('offers a test that never started when a stopped run cut none off', async () => {
    const output = runDir('SIGINT')
    const results = JSON.parse(
      fs.readFileSync(path.join(output, 'results.json'), 'utf8'),
    )
    // Stopped between tests: nothing interrupted, one test never started.
    results.suites[0].specs.splice(1, 1)
    fs.writeFileSync(path.join(output, 'results.json'), JSON.stringify(results))
    const line = await rerunLine(output, [])
    expect(line).toStartWith('Stopped by SIGINT before every test ran. Rerun')
    expect(line).toContain('(not run)')
    expect(line).toContain(
      'should not throw UnhandledPromiseRejection when page closes',
    )
  })

  it('says a stopped run stopped, and offers the test it cut off', async () => {
    const line = await rerunLine(runDir('SIGINT'), [])
    expect(line).not.toContain('Nothing failed')
    expect(line).toStartWith('Stopped by SIGINT before every test ran.')
    expect(line).toContain(
      "(interrupted): bun run test:neo-spec page-click --grep '(?:^| )page/page-click\\.spec\\.ts should click on a span with an inline element inside(?: @\\S+)*$'",
    )
  })
})

describe('--ci', () => {
  it('takes its own switches off the command line and leaves the rest for run.mjs', () => {
    expect(
      ciArgs(['--ci', '--workers', '16', '--grep', '--ci', '--write-expected']),
    ).toEqual({
      ci: true,
      writeExpected: true,
      expectedFrom: undefined,
      argv: ['--workers', '16', '--grep', '--ci'],
    })
    expect(ciArgs(['--expected-from', '123']).expectedFrom).toBe('123')
    expect(
      ciArgs(['--expected-from=https://github.com/o/r/actions/runs/9'])
        .expectedFrom,
    ).toBe('https://github.com/o/r/actions/runs/9')
    expect(() => ciArgs(['--write-expected'])).toThrow(/goes with --ci/)
    expect(() => ciArgs(['--expected-from'])).toThrow(/run ID or URL/)
    expect(() => ciArgs(['--expected-from', '1', 'page-goto'])).toThrow(
      /runs no tests/,
    )
  })

  it('knows a filtered run from the whole corpus', () => {
    expect(isFiltered([], ['--workers', '16', '--hosted-mode'])).toBe(false)
    expect(isFiltered(['page-goto'], [])).toBe(true)
    expect(isFiltered([], ['--grep', 'x'])).toBe(true)
    expect(isFiltered([], ['--shard=1/2'])).toBe(true)
  })

  it("reruns one test alone with the full run's other flags and the add-on it built", () => {
    const test = {
      file: 'tests/page/page-fill.spec.ts',
      titles: ['should fill (textarea)'],
    }
    expect(
      ciRerunArgs(
        [
          '--workers',
          '16',
          '-j8',
          '--hosted-mode',
          '--grep',
          'fill',
          '--skip-build',
        ],
        test,
      ),
    ).toEqual([
      '--hosted-mode',
      '--skip-build',
      '--workers',
      '1',
      'tests/page/page-fill.spec.ts',
      '--grep',
      exactGrep(test.file, test.titles),
    ])
    const grep = new RegExp(exactGrep(test.file, test.titles))
    expect(
      grep.test('browseros-neo page/page-fill.spec.ts should fill (textarea)'),
    ).toBe(true)
    expect(
      grep.test(
        'browseros-neo page/page-fill.spec.ts should fill (textarea) twice',
      ),
    ).toBe(false)
  })
})
