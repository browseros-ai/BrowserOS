/** The test:neo-spec wrapper: spec names, passed-through flags, and what it says when Node or the browser is missing. */
import { afterEach, describe, expect, it } from 'bun:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import {
  acquireLock,
  findBrowser,
  findNode,
  finish,
  releaseLock,
  rerunFlags,
  rerunLine,
  resolveSuites,
  runnerEnv,
  splitArgs,
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

describe('the preparation lock', () => {
  const dead = 4_000_001
  const stop = new Error('waited')

  it('a run that saw a dead owner cannot delete the lock another run took over', () => {
    const lock = path.join(tempDir(), 'source.lock')
    fs.mkdirSync(lock)
    fs.writeFileSync(path.join(lock, 'pid'), String(dead))
    let raced = false
    const late = () =>
      acquireLock(lock, {
        pid: 2,
        // Between this run reading the dead owner and acting on it, the other run takes over.
        isAlive: (pid) => {
          if (pid === dead && !raced) {
            raced = true
            acquireLock(lock, { pid: 1, isAlive: (other) => other === 1 })
          }
          return pid === 1
        },
        wait: () => {
          throw stop
        },
      })
    expect(late).toThrow(stop)
    expect(fs.readFileSync(path.join(lock, 'pid'), 'utf8')).toBe('1')
  })

  it('takes over a dead owner, waits for a live one, and retries a lock released meanwhile', () => {
    const lock = path.join(tempDir(), 'source.lock')
    fs.mkdirSync(lock)
    fs.writeFileSync(path.join(lock, 'pid'), String(dead))
    acquireLock(lock, { pid: 7, isAlive: () => false })
    expect(fs.readFileSync(path.join(lock, 'pid'), 'utf8')).toBe('7')
    let waits = 0
    acquireLock(lock, {
      pid: 8,
      isAlive: () => true,
      wait: () => {
        waits++
        releaseLock(lock, 7)
      },
    })
    expect(waits).toBe(1)
    expect(fs.readFileSync(path.join(lock, 'pid'), 'utf8')).toBe('8')
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

  it('says a stopped run stopped, and offers the test it cut off', async () => {
    const line = await rerunLine(runDir('SIGINT'), [])
    expect(line).not.toContain('Nothing failed')
    expect(line).toStartWith('Stopped by SIGINT before every test ran.')
    expect(line).toContain(
      "(interrupted): bun run test:neo-spec page-click --grep '(?:^| )page/page-click\\.spec\\.ts should click on a span with an inline element inside(?: @\\S+)*$'",
    )
  })
})
