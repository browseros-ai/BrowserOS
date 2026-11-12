/** The CI verdict end to end on hand-built result folders, with a fake run.mjs for the reruns. */
import { afterEach, beforeEach, describe, expect, it, spyOn } from 'bun:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { adoptLists, ciVerdict, pullExpected, type Runner } from './ci'

const title = 'a test the full run failed'

/** A Playwright JSON report with this one test, passed or failed. */
const report = (passed: boolean) => ({
  suites: [
    {
      title: 'page/page-goto.spec.ts',
      file: 'page/page-goto.spec.ts',
      specs: [
        {
          title,
          file: 'page/page-goto.spec.ts',
          tests: [
            {
              status: passed ? 'expected' : 'unexpected',
              results: [
                passed
                  ? { status: 'passed' }
                  : { status: 'failed', error: { message: 'boom' } },
              ],
            },
          ],
        },
      ],
    },
  ],
})
const score = { passed: 0, total: 1, skipped: 0, skippedByHarness: 0 }

let root: string
let summary: string | undefined
let quiet: ReturnType<typeof spyOn>
beforeEach(() => {
  // ciVerdict prints its summary and each rerun.
  quiet = spyOn(console, 'log').mockImplementation(() => {})
  root = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-ci-test-'))
  // A developer's shell inside a GitHub job must not get this test's summary.
  summary = process.env.GITHUB_STEP_SUMMARY
  delete process.env.GITHUB_STEP_SUMMARY
})
afterEach(() => {
  quiet.mockRestore()
  fs.rmSync(root, { recursive: true, force: true })
  if (summary !== undefined) process.env.GITHUB_STEP_SUMMARY = summary
})

/**
 * A finished full run in which the test failed, an empty committed list (so it is expected to
 * pass and is rerun), and reruns that each write the folder `attempts` describes.
 */
async function verdict(
  attempts: { results?: unknown; run: Record<string, unknown> }[],
) {
  const output = path.join(root, 'claw-full')
  fs.mkdirSync(output)
  const write = (dir: string, name: string, value: unknown) =>
    fs.writeFileSync(
      path.join(dir, name),
      typeof value === 'string' ? value : JSON.stringify(value),
    )
  write(output, 'results.json', report(false))
  write(output, 'scores.json', { all: score, inScope: score })
  write(output, 'run.json', { workers: 1, endedAt: 'now' })
  fs.copyFileSync(
    path.join(import.meta.dir, 'node/scope.json'),
    path.join(output, 'scope.json'),
  )
  const listFile = path.join(root, 'list.tsv')
  fs.writeFileSync(listFile, '# empty\n')
  let attempt = 0
  const runner: Runner = {
    node: 'node',
    env: {},
    flags: [],
    rerunArgs: () => [],
    runRunner: async (_node, env, _args, log) => {
      const folder = attempts[attempt++]
      const dir = path.join(env.NEO_NODE_OUTPUT_DIR ?? root, `claw-${attempt}`)
      fs.mkdirSync(dir, { recursive: true })
      if (folder.results !== undefined)
        write(dir, 'results.json', folder.results)
      write(dir, 'run.json', folder.run)
      if (log) fs.writeFileSync(log, '')
      return { code: 0, output: dir }
    },
  }
  const code = await ciVerdict({
    runner,
    output,
    complete: true,
    writeExpected: false,
    listFile,
  })
  return {
    code,
    attempts: attempt,
    verdict: JSON.parse(
      fs.readFileSync(path.join(output, 'verdict.json'), 'utf8'),
    ),
    summary: fs.readFileSync(path.join(output, 'summary.md'), 'utf8'),
  }
}

describe('ciVerdict', () => {
  it('passes a test that fails in the full run and passes a clean rerun', async () => {
    const result = await verdict([
      { results: report(true), run: { endedAt: 'now' } },
    ])
    expect(result.code).toBe(0)
    expect(result.verdict.flaky).toHaveLength(1)
  })

  it('fails the run when the passing rerun left its browsers behind', async () => {
    const result = await verdict([
      {
        results: report(true),
        run: {
          endedAt: 'now',
          cleanupError: 'Owned browser process group 42 survived cleanup',
        },
      },
    ])
    expect(result.verdict.flaky).toHaveLength(1)
    expect(result.code).toBe(1)
    expect(result.summary).toMatch(/run reported an error.*survived cleanup/)
  })

  it('fails the run when a rerun runner was killed before it finished', async () => {
    const result = await verdict([{ results: report(true), run: {} }])
    expect(result.code).toBe(1)
    expect(result.summary).toMatch(/did not finish/)
  })

  it("keeps an unreadable rerun report's runner errors, even when the next rerun passes", async () => {
    const result = await verdict([
      {
        results: '{ not json',
        run: { endedAt: 'now', error: 'launcher failed' },
      },
      { results: report(true), run: { endedAt: 'now' } },
    ])
    expect(result.attempts).toBe(2)
    expect(result.verdict.flaky).toHaveLength(1)
    expect(result.code).toBe(1)
    expect(result.summary).toMatch(/results\.json is not JSON/)
    expect(result.summary).toMatch(/launcher failed/)
  })
})

describe('adoptLists (--expected-from)', () => {
  const line = (title: string) =>
    `-\tfail\ttests/page/page-goto.spec.ts\t${title}`
  const entry = (title: string) => ({
    id: null,
    outcome: 'fail',
    file: 'tests/page/page-goto.spec.ts',
    title,
  })
  /** A downloaded artifact: the new list and the proof its run wrote, judged against `base`. */
  function artifact(after: string[], base: string[], removed: unknown[]) {
    const dir = path.join(root, 'artifact')
    fs.mkdirSync(dir)
    fs.writeFileSync(
      path.join(dir, 'linux-x64.tsv'),
      `# list\n${after.map(line).join('\n')}\n`,
    )
    fs.writeFileSync(
      path.join(dir, 'linux-x64.proof.json'),
      JSON.stringify({ schemaVersion: 1, base: base.map(entry), removed }),
    )
    return dir
  }
  const target = () => path.join(root, 'committed.tsv')
  const quietErrors = () => spyOn(console, 'error').mockImplementation(() => {})

  it('takes a list whose dropped test passed the full run and three of three alone', async () => {
    const dir = artifact(
      ['kept'],
      ['kept', 'fixed'],
      [
        {
          ...entry('fixed'),
          evidence: {
            kind: 'passed',
            fullRun: 'pass',
            reruns: ['pass', 'pass', 'pass'],
          },
        },
      ],
    )
    expect(await adoptLists(dir, { target, mainList: () => undefined })).toBe(0)
    expect(fs.readFileSync(target(), 'utf8')).toContain('\tkept')
  })

  it('refuses, and writes nothing, when a dropped test has only two passing reruns', async () => {
    const errors = quietErrors()
    const dir = artifact(
      ['kept'],
      ['kept', 'lucky'],
      [
        {
          ...entry('lucky'),
          evidence: {
            kind: 'passed',
            fullRun: 'pass',
            reruns: ['pass', 'pass'],
          },
        },
      ],
    )
    expect(await adoptLists(dir, { target, mainList: () => undefined })).toBe(1)
    expect(fs.existsSync(target())).toBe(false)
    expect(String(errors.mock.calls[0][0])).toMatch(
      /lucky: full run pass, alone pass, pass; leaving the list needs a passing full run and 3 of 3/,
    )
    errors.mockRestore()
  })

  it('refuses a run that wrote no proof', async () => {
    const errors = quietErrors()
    const dir = artifact(['kept'], ['kept'], [])
    fs.rmSync(path.join(dir, 'linux-x64.proof.json'))
    expect(await adoptLists(dir, { target, mainList: () => undefined })).toBe(1)
    expect(String(errors.mock.calls[0][0])).toMatch(/predates proofs/)
    errors.mockRestore()
  })
})

describe('the update command against the checkout and main', () => {
  const list = (titles: string[]) =>
    `# list\n${titles.map((title) => `-\tfail\ttests/page/page-goto.spec.ts\t${title}`).join('\n')}\n`
  const proof = (base: string[]) =>
    JSON.stringify({
      schemaVersion: 1,
      base: base.map((title) => ({
        id: null,
        outcome: 'fail',
        file: 'tests/page/page-goto.spec.ts',
        title,
      })),
      removed: [],
    })
  const committed = () => path.join(root, 'committed.tsv')
  const fakeRun =
    (after: string[], base: string[]) =>
    (_id: string, _repo: string | undefined, dir: string) => {
      fs.writeFileSync(path.join(dir, 'linux-x64.tsv'), list(after))
      fs.writeFileSync(path.join(dir, 'linux-x64.proof.json'), proof(base))
    }

  it('downloads, then adopts before the download is cleaned up', async () => {
    fs.writeFileSync(committed(), list(['kept']))
    const code = await pullExpected('https://github.com/o/r/actions/runs/7', {
      download: fakeRun(['kept', 'new'], ['kept']),
      target: committed,
      mainList: () => list(['kept']),
    })
    expect(code).toBe(0)
    expect(fs.readFileSync(committed(), 'utf8')).toContain('\tnew')
  })

  it('refuses a run that never judged a test main still lists, so an artifact cannot erase it', async () => {
    const errors = spyOn(console, 'error').mockImplementation(() => {})
    fs.writeFileSync(committed(), list(['kept']))
    const code = await pullExpected('7', {
      download: fakeRun([], []),
      target: committed,
      mainList: () => list(['kept']),
    })
    expect(code).toBe(1)
    expect(fs.readFileSync(committed(), 'utf8')).toContain('\tkept')
    expect(String(errors.mock.calls[0][0])).toMatch(
      /kept: the run did not judge it .* origin\/main still lists it/,
    )
    errors.mockRestore()
  })

  it('lets go of a test the run never judged when main already dropped it', async () => {
    fs.writeFileSync(committed(), list(['kept', 'fixed on main']))
    const code = await pullExpected('7', {
      download: fakeRun(['kept'], ['kept']),
      target: committed,
      mainList: () => list(['kept']),
    })
    expect(code).toBe(0)
    expect(fs.readFileSync(committed(), 'utf8')).not.toContain('fixed on main')
  })
})

describe('known-flaky.tsv', () => {
  it('parses the committed file, every entry with a reason and a tracked fix', async () => {
    const { readKnownFlaky, parseKnownFlaky } = await import(
      './node/known-flaky.mjs'
    )
    const entries = [...readKnownFlaky().values()]
    expect(entries.map((entry) => entry.id)).toEqual(['ER-006', 'SS-054'])
    for (const entry of entries) {
      expect(entry.reason.length).toBeGreaterThan(20)
      expect(entry.tracked.length).toBeGreaterThan(10)
    }
    expect(() =>
      parseKnownFlaky('ER-006\ttests/page/a.spec.ts\tt\treason\t'),
    ).toThrow(/each filled in/)
  })
})
