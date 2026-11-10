import assert from 'node:assert/strict'
import fs from 'node:fs'
import { createRequire } from 'node:module'
import test from 'node:test'
import vm from 'node:vm'
import { source } from '../../js/prepare.mjs'
import { classifyCase } from './classify.mjs'
import { clawRefusals } from './refusals-claw.mjs'

function classify(message, engine = 'claw', extra = {}) {
  return classifyCase(
    {
      suite: 'tests/page/example.spec.ts',
      titles: ['example'],
      test: {
        status: 'unexpected',
        results: [{ status: 'failed', errors: [{ message, ...extra }] }],
      },
    },
    undefined,
    { engine },
  )
}

test('tests that claw refusals are selected by engine and preserve error precedence', () => {
  const message =
    'Error: browser.newContext: Neo has one browser profile; open a tab with context.newPage()'
  assert.equal(classify(message).verdict, 'neo-lacks')
  assert.equal(classify(message, 'neo').verdict, 'neo-differs')
  assert.equal(
    classify(`Error: expectLoop: toHaveCount; read failed: ${message}`).verdict,
    'neo-lacks',
  )
  assert.equal(
    classify('Error: page.click: neo / unsupported: cannot act on linked tabs')
      .verdict,
    'neo-lacks',
  )
  assert.equal(
    classify('Error: page.click: expected number').verdict,
    'neo-differs',
  )
  assert.equal(classify(`Expected: ${message}`).verdict, 'neo-differs')
})

test('tests that all listed v1 unsupported members and events have refusal evidence', () => {
  const require = createRequire(`${source}/package.json`)
  const ts = require('typescript')
  const text = fs.readFileSync(
    new URL(
      '../../../neo-playwright-js/src/prelude/unsupported.ts',
      import.meta.url,
    ),
    'utf8',
  )
  const js = ts.transpileModule(text, {
    compilerOptions: { module: ts.ModuleKind.CommonJS },
  }).outputText
  const exports = {}
  vm.compileFunction(js, ['exports', 'require'])(exports, () => ({}))
  for (const [owner, members] of Object.entries(exports.UNSUPPORTED)) {
    for (const [member, entry] of Object.entries(members)) {
      const api = `${owner[0].toLowerCase() + owner.slice(1)}.${member}`
      const message = `Error: ${api}: ${typeof entry === 'string' ? entry : entry.hint}`
      assert.equal(classify(message).verdict, 'neo-lacks', message)
    }
  }
  const events = fs.readFileSync(
    new URL(
      '../../../neo-playwright-js/src/prelude/events.ts',
      import.meta.url,
    ),
    'utf8',
  )
  const unreported = vm.runInNewContext(
    `(${events.match(/const UNREPORTED[^=]*= (\{[\s\S]*?\n\})/)[1]})`,
  )
  for (const [event, hint] of Object.entries(unreported))
    assert.equal(
      classify(`Error: page.on('${event}'): ${hint}`).verdict,
      'neo-lacks',
    )
  for (const refusal of clawRefusals)
    for (const source of refusal.sources)
      assert.ok(
        fs.existsSync(new URL(`../../../${source}`, import.meta.url)),
        source,
      )
})

test('tests that absent v1 namespaces are lacks while unrelated undefined values are differences', () => {
  for (const [property, snippet] of [
    ['launch', 'await browserType.launch()'],
    ['newContext', 'await playwright.request.newContext()'],
    ['connect', 'await playwright.chromium.connect()'],
  ]) {
    const message = `TypeError: Cannot read properties of undefined (reading '${property}')`
    assert.equal(classify(message, 'claw', { snippet }).verdict, 'neo-lacks')
    assert.equal(classify(message, 'neo', { snippet }).verdict, 'neo-differs')
  }
  assert.equal(
    classify(
      "TypeError: Cannot read properties of undefined (reading 'launch')",
      'claw',
      { snippet: 'await custom.launch()' },
    ).verdict,
    'neo-differs',
  )
})

test('tests that claw setup and add-on failures belong to the harness', () => {
  for (const message of [
    'Error: Cannot find module /runner/claw-addon/claw_node.node',
    'Error: dlopen(/runner/claw_node.node): image not found',
    'Timeout of 60000ms exceeded while setting up fixture "__claw".',
  ])
    assert.equal(classify(message).verdict, 'harness-error', message)
})
