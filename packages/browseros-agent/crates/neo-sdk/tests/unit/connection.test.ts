/**
 * The reply half of Neo's wire against Playwright's recorded errors. Rust encodes each golden's
 * reply from its own errors, call-log folding and envelope (`neo-runtime/src/dispatch.rs`, which
 * writes `neo-runtime/tests/fixtures/replies.json`); this rebuilds each into the error a script
 * sees and compares it with what Playwright's client threw.
 */

import { describe, expect, test } from 'bun:test'
import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import {
  agentLocation,
  formatCallLog,
  rebuildError,
} from '../../src/connection'
import { TimeoutError } from '../../src/errors'

type Golden = {
  name: string
  error?: { name: string; message: string; instanceOfTimeoutError: boolean }
}

const replies: Record<string, string> = JSON.parse(
  readFileSync(
    join(import.meta.dir, '../../../neo-runtime/tests/fixtures/replies.json'),
    'utf8',
  ),
)
const goldens: Golden[] = JSON.parse(
  readFileSync(join(import.meta.dir, '../goldens/click-goldens.json'), 'utf8'),
).cases

function reply(name: string) {
  const text = replies[name]
  if (text === undefined) throw new Error(`replies.json has no ${name} reply`)
  return JSON.parse(text)
}

describe('rebuildError', () => {
  const failed = goldens.flatMap(({ name, error }) =>
    error ? [{ name, error }] : [],
  )

  test('covers every golden that threw', () => {
    expect(failed.map((golden) => golden.name).sort()).toEqual([
      'covered',
      'disabled',
      'hidden',
      'strict',
    ])
  })

  for (const golden of failed) {
    test(`Neo's ${golden.name} reply rebuilds Playwright's error`, () => {
      const { error, log } = reply(golden.name)
      const rebuilt = rebuildError('locator.click', error.error, log)
      expect({
        name: rebuilt.name,
        message: rebuilt.message,
        instanceOfTimeoutError: rebuilt instanceof TimeoutError,
      }).toEqual({
        name: golden.error.name,
        message: golden.error.message,
        instanceOfTimeoutError: golden.error.instanceOfTimeoutError,
      })
      expect(rebuilt.log).toEqual(log)
    })
  }

  test('a closed target reads as a plain Error, as in Playwright', () => {
    const { error, log } = reply('targetClosed')
    const rebuilt = rebuildError('page.click', error.error, log)
    expect([rebuilt.name, rebuilt.message, rebuilt.log]).toEqual([
      'Error',
      'page.click: Target page, context or browser has been closed',
      [],
    ])
  })

  test('a successful click replies with no result', () => {
    expect(reply('ok')).toEqual({ id: 1 })
  })

  test('formatCallLog prints nothing for an absent or blank log', () => {
    expect([
      formatCallLog(undefined),
      formatCallLog([]),
      formatCallLog(['']),
    ]).toEqual(['', '', ''])
  })
})

// Stack text exactly as probe P4 recorded it from neo-runtime's realm.
describe('agentLocation', () => {
  test('names the innermost frame of the agent script, minus the wrapper lines', () => {
    expect(agentLocation('    at anonymous (<input>:3:12)\n', 2)).toEqual({
      file: 'run.js',
      line: 1,
      column: 12,
    })
    expect(
      agentLocation(
        '    at inner (<input>:4:14)\n    at anonymous (<input>:6:8)\n',
        2,
      ),
    ).toEqual({ file: 'run.js', line: 2, column: 14 })
    expect(
      agentLocation(
        '    at <anonymous> (<input>:3:68)\n    at <anonymous> (eval_script:8:30)\n',
        2,
      ),
    ).toEqual({ file: 'run.js', line: 1, column: 68 })
  })

  test('skips the SDK and native frames', () => {
    const stack = [
      '    at captureLocation (eval_script:301:19)',
      '    at send (eval_script:290:22)',
      '    at forEach (native)',
      '    at click (eval_script:180:12)',
      '    at anonymous (<input>:4:17)',
      '',
    ].join('\n')
    expect(agentLocation(stack, 2)).toEqual({
      file: 'run.js',
      line: 2,
      column: 17,
    })
  })

  test('keeps only the file when no frame is the agent script', () => {
    expect(agentLocation('    at click (eval_script:180:12)\n', 2)).toEqual({
      file: 'run.js',
    })
    expect(agentLocation('', 2)).toEqual({ file: 'run.js' })
  })
})
