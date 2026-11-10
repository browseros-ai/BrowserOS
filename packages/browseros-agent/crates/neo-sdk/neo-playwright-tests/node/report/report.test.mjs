/** Neo: CLI tests verify readable output and JSONL against the same recorded verdicts. */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { flattenResults } from './classify.mjs'
import { commandLine, renderReport, runnerCommand } from './report.mjs'

function fixture(name) {
  return JSON.parse(
    fs.readFileSync(
      new URL(`./fixtures/${name}.json`, import.meta.url),
      'utf8',
    ),
  )
}

function metadata(mode) {
  return {
    mode,
    argv: [
      'node',
      'node/run.mjs',
      ...(mode === 'reference' ? ['--reference'] : []),
      'page-goto.spec.ts',
    ],
    workers: 1,
    startedAt: '2026-11-10T00:00:00Z',
    endedAt: '2026-11-10T00:01:00Z',
    commit: 'fixture-commit',
    sdkSha256: 'fixture-sdk',
    specsVerified: true,
    contextOptionOmissions: [],
  }
}

function inputs() {
  const neo = {
    results: fixture('neo'),
    metadata: metadata('neo'),
    directory: '/fixtures/neo',
  }
  const reference = {
    results: fixture('reference'),
    metadata: metadata('reference'),
    directory: '/fixtures/reference',
  }
  const bridge = flattenResults(neo.results).map((row) => ({
    key: row.key,
    suite: row.suite,
    verdict: 'BridgeLimit',
  }))
  return { neo, reference, bridge }
}

test('report exposes commands, actual counts, transitions, gaps and _expect limits', () => {
  const report = renderReport(inputs())
  assert.match(report.markdown, /node node\/run.mjs page-goto.spec.ts/)
  assert.match(
    report.markdown,
    /node node\/run.mjs --reference page-goto.spec.ts/,
  )
  assert.match(
    report.markdown,
    /\| tests\/page\/page-goto.spec.ts \| 1 \| 1 \| 0 \| 3 \| 0 \| 0 \| 0 \|/,
  )
  assert.match(
    report.markdown,
    /\*\*5 now have a plain Playwright pass\/fail\*\*/,
  )
  assert.match(report.markdown, /\| Locator._expect \| 2 \|/)
  assert.match(report.markdown, /\*\*2 cases\*\* have only observed/)
  assert.match(report.markdown, /not proof that later assertions would pass/)
  assert.match(report.markdown, /Page.setExtraHTTPHeaders \(created context\)/)
  assert.ok(!report.markdown.includes('\x1b'))
})

test('runner errors are visible without fabricating per-case transitions', () => {
  const input = inputs()
  input.neo.results.errors.push({
    message: 'Error: worker teardown failed',
    location: { file: 'fixtures.ts', line: 5, column: 1 },
  })
  const report = renderReport(input)
  assert.match(report.markdown, /\*\*Run incomplete:\*\*/)
  assert.match(
    report.markdown,
    /\| neo \| Error: worker teardown failed \| fixtures.ts:5:1 \|/,
  )
  assert.equal(report.counts.pass, 1)
})

test('recorded argv is shell-quoted without interpreting command substitutions', () => {
  const args = [
    'node',
    '/some path/run.mjs',
    '--grep',
    "it's $(unsafe) `literal`",
  ]
  const command = commandLine(args)
  const result = spawnSync(
    '/bin/sh',
    ['-c', `set -- ${command}; printf '%s\\n' "$@"`],
    { encoding: 'utf8' },
  )
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(result.stdout.trimEnd().split('\n'), args)
})

test('CLI writes markdown and parseable one-verdict-per-case JSONL; invalid inputs leave no report', (t) => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'neo-report-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  const input = inputs()
  for (const mode of ['neo', 'reference']) {
    fs.mkdirSync(path.join(root, mode))
    fs.writeFileSync(
      path.join(root, mode, 'results.json'),
      JSON.stringify(input[mode].results),
    )
    fs.writeFileSync(
      path.join(root, mode, 'run.json'),
      JSON.stringify(input[mode].metadata),
    )
  }
  const bridgePath = path.join(root, 'cases.jsonl')
  fs.writeFileSync(
    bridgePath,
    input.bridge.map((row) => JSON.stringify(row)).join('\n'),
  )
  const output = path.join(root, 'report.md')
  const json = path.join(root, 'verdicts.jsonl')
  const cli = new URL('./report.mjs', import.meta.url)
  const args = [
    cli.pathname,
    '--neo',
    path.join(root, 'neo'),
    '--reference',
    path.join(root, 'reference'),
    '--bridge',
    bridgePath,
    '--out',
    output,
    '--json',
    json,
  ]
  const result = spawnSync(process.execPath, args, { encoding: 'utf8' })
  assert.equal(result.status, 0, result.stderr)
  assert.equal(JSON.parse(result.stdout).cases, 5)
  const rows = fs
    .readFileSync(json, 'utf8')
    .trim()
    .split('\n')
    .map((line) => JSON.parse(line))
  assert.equal(new Set(rows.map((row) => row.key)).size, 5)
  assert.equal(
    rows.filter((row) => row.verdict === 'needs-internals').length,
    3,
  )
  assert.match(fs.readFileSync(output, 'utf8'), /## Per suite/)
  fs.rmSync(path.join(root, 'neo', 'run.json'))
  fs.rmSync(output)
  const failure = spawnSync(process.execPath, args, { encoding: 'utf8' })
  assert.equal(failure.status, 1)
  assert.match(failure.stderr, /Cannot read .*run.json/)
  assert.ok(!fs.existsSync(output))
})

test('runner argv without a command renders the runner, and empty argv is the full corpus', () => {
  const runner =
    'node packages/browseros-agent/crates/neo-sdk/neo-playwright-tests/node/run.mjs'
  assert.equal(runnerCommand([]), runner)
  assert.equal(runnerCommand(['--reference']), `${runner} --reference`)
  assert.equal(runnerCommand(['node', 'x.mjs']), 'node x.mjs')
})
