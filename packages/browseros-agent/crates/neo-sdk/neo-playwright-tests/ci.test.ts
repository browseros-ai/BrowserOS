/** The CI verdict end to end on hand-built result folders, with a fake run.mjs for the reruns. */
import { afterEach, beforeEach, describe, expect, it, spyOn } from 'bun:test'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { ciVerdict, type Runner } from './ci'

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
