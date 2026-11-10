/** Neo: Drift, private-copy boundaries, selection and skip refresh must fail or act explicitly. */
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { corpus, source } from '../js/prepare.mjs'
import {
  applyManifest,
  collectCases,
  curationArgs,
  entryKey,
  patchReason,
  refreshSkips,
  skipReason,
  summarize,
  validateManifest,
} from './curation.mjs'
import { runnerOptions } from './options.mjs'
import { materialize } from './overlay.mjs'
import { flattenResults } from './report/classify.mjs'

const file = 'tests/page/page-click.spec.ts'
const row = {
  suite: file,
  caseId: 'CL-001',
  title: 'CL-001 works',
  titles: ['CL-001 works'],
  location: { line: 1, column: 1 },
}
const original =
  "it('CL-001 works', () => { expect(error.message).toBe('wording'); expect(error.name).toBe('TypeError'); });\n"
const patch = { find: "expect(error.message).toBe('wording');", replace: '' }
const entry = {
  file,
  id: row.caseId,
  title: row.title,
  action: 'patch',
  reason: patchReason,
  patches: [patch],
}
const manifest = (entries) => ({
  schemaVersion: 1,
  pin: corpus.revision,
  entries,
})
const prepared = {
  skip: !fs.existsSync(path.join(source, 'node_modules/typescript')),
}
function scratch(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'curation-test-'))
  t.after(() => fs.rmSync(root, { recursive: true, force: true }))
  return root
}

test(
  'only the private callback is patched, preserving its error class and source',
  prepared,
  (t) => {
    const root = scratch(t)
    const target = path.join(root, file)
    fs.mkdirSync(path.dirname(target), { recursive: true })
    const text =
      original +
      "it('CL-002 other', () => { expect(error.message).toBe('wording'); });\n"
    fs.writeFileSync(target, text)
    const changes = applyManifest(manifest([entry]), [row], root)
    assert.equal(fs.readFileSync(target, 'utf8'), text.replace(patch.find, ''))
    assert.match(fs.readFileSync(target, 'utf8'), /error.name/)
    assert.notEqual(changes[0].beforeSha256, changes[0].afterSha256)
    assert.equal(original.includes(patch.find), true)
  },
)

test(
  'pin, file, ID, title, duplicate entry and callback drift are rejected',
  prepared,
  () => {
    for (const bad of [
      { ...manifest([entry]), pin: 'different' },
      { ...manifest([entry]), schemaVersion: 2 },
      manifest([{ ...entry, file: '../escape.spec.ts' }]),
      manifest([{ ...entry, id: 'CL-099' }]),
      manifest([{ ...entry, title: 'renamed' }]),
      manifest([entry, entry]),
      manifest([{ ...entry, action: 'ignore' }]),
      manifest([{ ...entry, patches: [] }]),
    ])
      assert.throws(() => validateManifest(bad, [row], () => original))
    assert.throws(
      () =>
        validateManifest(
          manifest([entry]),
          [{ ...row, location: { line: 2, column: 1 } }],
          () => original,
        ),
      /callback drift/,
    )
  },
)

test(
  'missing, repeated and overlapping find strings fail before any write',
  prepared,
  (t) => {
    const root = scratch(t)
    const target = path.join(root, file)
    fs.mkdirSync(path.dirname(target), { recursive: true })
    fs.writeFileSync(target, original)
    for (const patches of [
      [{ find: 'missing', replace: '' }],
      [{ find: 'expect', replace: '' }],
      [patch, { find: 'error.message', replace: 'error.name' }],
      [{ find: '', replace: '' }],
      [{ find: patch.find, replace: 42 }],
    ]) {
      assert.throws(() =>
        applyManifest(manifest([{ ...entry, patches }]), [row], root),
      )
      assert.equal(fs.readFileSync(target, 'utf8'), original)
    }
    assert.throws(
      () =>
        applyManifest(
          manifest([
            { ...entry, patches: [{ find: patch.find, replace: '}' }] },
          ]),
          [row],
          root,
        ),
      /Cannot parse/,
    )
    assert.equal(fs.readFileSync(target, 'utf8'), original)
  },
)

test(
  'shared parameterized callbacks require consistent decisions for all surviving cases',
  prepared,
  () => {
    const second = {
      ...row,
      title: 'CL-001 parameter 2',
      titles: ['CL-001 parameter 2'],
    }
    const secondEntry = { ...entry, title: second.title }
    assert.throws(
      () => validateManifest(manifest([entry]), [row, second], () => original),
      /Shared callback/,
    )
    assert.throws(
      () =>
        validateManifest(
          manifest([
            entry,
            { ...secondEntry, patches: [{ ...patch, replace: ';' }] },
          ]),
          [row, second],
          () => original,
        ),
      /Conflicting/,
    )
    assert.doesNotThrow(() =>
      validateManifest(
        manifest([entry, secondEntry]),
        [row, second],
        () => original,
      ),
    )
    assert.doesNotThrow(() =>
      validateManifest(
        manifest([
          entry,
          { ...secondEntry, action: 'delete', patches: undefined },
        ]),
        [row, second],
        () => original,
      ),
    )
  },
)

test(
  'prepared source, symlinks and hard links cannot be patch targets',
  prepared,
  (t) => {
    assert.throws(
      () => applyManifest(manifest([]), [], source),
      /prepared source/,
    )
    const root = scratch(t)
    const outside = scratch(t)
    const target = path.join(root, file)
    const external = path.join(outside, 'original.ts')
    fs.mkdirSync(path.dirname(target), { recursive: true })
    fs.writeFileSync(external, original)
    fs.symlinkSync(external, target)
    assert.throws(
      () => applyManifest(manifest([entry]), [row], root),
      /private copy/,
    )
    fs.unlinkSync(target)
    fs.linkSync(external, target)
    assert.throws(
      () => applyManifest(manifest([entry]), [row], root),
      /hard-linked/,
    )
    assert.equal(fs.readFileSync(external, 'utf8'), original)
  },
)

test('no-curation is consumed by the launcher and preserves other selection', () => {
  assert.equal(runnerOptions([]).curation, true)
  const parsed = runnerOptions(['--no-curation', '--list'])
  assert.equal(parsed.curation, false)
  assert.ok(parsed.args.includes('--list'))
  assert.ok(!parsed.args.includes('--no-curation'))
})

test('deletion selection composes caller exclusions and handles trailing title whitespace', (t) => {
  const output = scratch(t)
  const deletion = {
    ...entry,
    action: 'delete',
    title: 'CL-001 trailing ',
    patches: undefined,
  }
  fs.writeFileSync(
    path.join(output, 'curation.json'),
    JSON.stringify(manifest([deletion])),
  )
  fs.writeFileSync(
    path.join(output, 'curation-upstream.json'),
    JSON.stringify({
      suites: [
        {
          title: 'page/page-click.spec.ts',
          file: 'page/page-click.spec.ts',
          specs: [
            {
              title: deletion.title,
              file: 'page/page-click.spec.ts',
              tests: [],
            },
          ],
        },
      ],
    }),
  )
  const ownList = path.join(output, 'own.txt')
  fs.writeFileSync(ownList, 'page/other.spec.ts › other test')
  const args = curationArgs(
    [
      '--grep',
      'CL-001',
      '--grep-invert',
      'caller pattern',
      '--test-list-invert',
      ownList,
      '--',
      'page-click.spec.ts',
    ],
    output,
  )
  assert.ok(args.indexOf('--test-list-invert') < args.indexOf('--'))
  assert.ok(args.indexOf('--grep-invert') < args.indexOf('--'))
  const reject = new RegExp(args[args.indexOf('--grep-invert') + 1])
  assert.ok(
    reject.test('browseros-neo page/page-click.spec.ts CL-001 trailing '),
  )
  assert.ok(reject.test('caller pattern'))
  assert.ok(
    !reject.test('browseros-neo page/page-click.spec.ts CL-001 trailing other'),
  )
  assert.match(
    fs.readFileSync(args[args.indexOf('--test-list-invert') + 1], 'utf8'),
    /other test/,
  )
  assert.ok(args.includes('--grep'))
})

test('skip lookup uses the file and full describe path, retaining its reason', () => {
  const skip = {
    ...entry,
    action: 'skip',
    title: 'nested › CL-001 works',
    reason: 'needs Playwright internals: Locator._expect',
  }
  assert.equal(
    skipReason([skip], file, ['nested', 'CL-001 works']),
    skip.reason,
  )
  assert.equal(skipReason([skip], file, ['CL-001 works']), undefined)
  assert.equal(
    skipReason([skip], 'elsewhere.ts', ['nested', 'CL-001 works']),
    undefined,
  )
})

test('skip refresh is idempotent, preserves deletions and patches, and only refreshes supplied cases', () => {
  const deleted = {
    ...entry,
    title: 'deleted',
    action: 'delete',
    patches: undefined,
  }
  const unrelated = {
    ...entry,
    title: 'unrelated',
    action: 'skip',
    reason: 'manual decision',
    patches: undefined,
  }
  const input = manifest([entry, deleted, unrelated])
  const verdict = {
    ...row,
    verdict: 'needs-internals',
    gapKey: 'Locator._expect',
  }
  const refreshed = refreshSkips(input, [
    verdict,
    { ...verdict, title: deleted.title },
  ])
  const skip = refreshed.entries.find((e) => entryKey(e) === entryKey(entry))
  assert.equal(skip.action, 'skip')
  assert.equal(skip.reason, 'needs Playwright internals: Locator._expect')
  assert.deepEqual(skip.patches, entry.patches)
  assert.deepEqual(
    refreshed.entries.find((e) => e.title === deleted.title),
    deleted,
  )
  assert.deepEqual(
    refreshed.entries.find((e) => e.title === unrelated.title),
    unrelated,
  )
  assert.deepEqual(refreshSkips(refreshed, [verdict]), refreshed)
  assert.deepEqual(
    refreshSkips(refreshed, [{ ...verdict, verdict: 'pass' }]).entries.find(
      (e) => e.title === entry.title,
    ),
    entry,
  )
  assert.throws(() => refreshSkips(input, [verdict, verdict]), /Duplicate/)
  assert.throws(
    () => refreshSkips(input, [{ ...verdict, gapKey: null }]),
    /Missing internals gap/,
  )
})

test('summary partitions every action and reason', () => {
  const summary = summarize([
    entry,
    { ...entry, action: 'delete', reason: 'invalid-here' },
    { ...entry, action: 'skip', reason: 'internals' },
  ])
  assert.deepEqual(summary.delete, {
    total: 1,
    byReason: { 'invalid-here': 1 },
  })
  assert.deepEqual(summary.skip, { total: 1, byReason: { internals: 1 } })
  assert.equal(summary.patch.total, 1)
})

test('the CLI also runs when invoked through a symlink', (t) => {
  const link = path.join(scratch(t), 'curation.mjs')
  fs.symlinkSync(
    fileURLToPath(new URL('./curation.mjs', import.meta.url)),
    link,
  )
  const result = spawnSync(process.execPath, [link, 'unknown-command'], {
    encoding: 'utf8',
  })
  assert.equal(result.status, 1)
  assert.match(result.stderr, /Usage: node curation.mjs skip-from-verdicts/)
})

test(
  'real Playwright selection and the auto fixture delete/skip exact titles without a browser',
  prepared,
  async (t) => {
    const output = scratch(t)
    const root = path.join(output, 'upstream')
    const { configPath } = materialize(root, {
      reference: true,
      output,
      curation: false,
    })
    const { default: config } = await import(pathToFileURL(configPath).href)
    config.testMatch = ['curation-probe.spec.ts']
    config.workers = 1
    fs.writeFileSync(configPath, `export default ${JSON.stringify(config)};\n`)
    const probe = 'tests/curation-probe.spec.ts'
    fs.writeFileSync(
      path.join(root, probe),
      `
import { test as base, expect } from ${JSON.stringify(fileURLToPath(new URL('./fixtures.mjs', import.meta.url)))};
const test = base.extend({ __neoWorker: [async ({}, use) => { throw new Error('must not launch a browser'); }, { scope: 'worker' }] });
test.describe('first', () => test('CU-001 shared', () => { throw new Error('must skip'); }));
test.describe('second', () => test('CU-001 shared', () => { expect(true).toBe(true); }));
test('CU-002 deleted ', () => { throw new Error('must delete'); });
test('CU-002 deleted', () => { expect(true).toBe(true); });
`,
    )
    collectCases(root, configPath, output)
    const reason = 'needs Playwright internals: test gap'
    const snapshot = path.join(output, 'curation.json')
    fs.writeFileSync(
      snapshot,
      JSON.stringify(
        manifest([
          {
            file: probe,
            id: 'CU-001',
            title: 'first › CU-001 shared',
            action: 'skip',
            reason,
          },
          {
            file: probe,
            id: 'CU-002',
            title: 'CU-002 deleted ',
            action: 'delete',
            reason: 'diagnostics',
          },
        ]),
      ),
    )
    const result = spawnSync(
      process.execPath,
      [
        path.join(source, 'packages/playwright/cli.js'),
        'test',
        '--config',
        configPath,
        ...curationArgs([], output),
      ],
      {
        cwd: root,
        encoding: 'utf8',
        timeout: 60_000,
        env: {
          ...process.env,
          NEO_NODE_OVERLAY: root,
          NEO_NODE_OUTPUT: output,
          NEO_NODE_MODE: 'reference',
          NEO_NODE_CURATION: snapshot,
          PWTEST_UNDER_TEST: '1',
          PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: '1',
        },
      },
    )
    assert.equal(
      result.status,
      0,
      result.error?.message || result.stdout + result.stderr,
    )
    const cases = flattenResults(
      JSON.parse(fs.readFileSync(path.join(output, 'results.json'), 'utf8')),
    )
    assert.equal(cases.length, 3)
    const skipped = cases.find((row) => row.title === 'first › CU-001 shared')
    assert.equal(skipped.test.status, 'skipped')
    assert.ok(
      skipped.test.annotations.some(
        (annotation) =>
          annotation.type === 'skip' && annotation.description === reason,
      ),
    )
    assert.equal(
      cases.find((row) => row.title === 'second › CU-001 shared').test.status,
      'expected',
    )
    assert.equal(
      cases.find((row) => row.title === 'CU-002 deleted').test.status,
      'expected',
    )
    assert.ok(
      !fs.readdirSync(output).some((name) => name.startsWith('worker-')),
    )
  },
)

test('skip refresh adds uncovered expectLoop checks, keeps applied skips, and restores a patch reason', () => {
  const patched = { ...entry, reason: 'error wording: keep that it fails' }
  const input = manifest([patched])
  const uncovered = {
    ...row,
    verdict: 'skip',
    reason: 'expectLoop: toHaveRole is not covered',
  }
  const refreshed = refreshSkips(input, [uncovered])
  const skip = refreshed.entries[0]
  assert.equal(skip.action, 'skip')
  assert.equal(skip.reason, 'expectLoop: toHaveRole is not covered')
  assert.equal(skip.patchReason, 'error wording: keep that it fails')
  // A later run with this manifest applied reports the skip with the manifest's reason.
  assert.deepEqual(refreshSkips(refreshed, [uncovered]), refreshed)
  const internals = {
    ...row,
    verdict: 'skip',
    reason: 'needs Playwright internals: toImpl',
  }
  const internal = refreshSkips(input, [
    { ...row, verdict: 'needs-internals', gapKey: 'toImpl' },
  ])
  assert.deepEqual(refreshSkips(internal, [internals]), internal)
  // Once the check passes, the patch returns with its own reason.
  assert.deepEqual(
    refreshSkips(refreshed, [{ ...row, verdict: 'pass' }]).entries[0],
    patched,
  )
})
