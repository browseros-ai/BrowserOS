/** Neo: The upstream test-only builtins fixture retains native bindings in every document. */
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { createContext, runInContext } from 'node:vm'
import { installTestBuiltins } from './test-builtins.mjs'

test('test builtins capture bound browser functions and native objects in the main world', async () => {
  const calls = []
  const contexts = [createContext({}), createContext({})]
  for (const context of contexts)
    runInContext(
      `
      globalThis.originals = {performance: {}, Intl, Date, AbortSignal: {}, Temporal: {}};
      Object.assign(globalThis, originals);
      for (const name of ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval',
        'requestAnimationFrame', 'cancelAnimationFrame', 'requestIdleCallback', 'cancelIdleCallback'])
        globalThis[name] = function (...args) { return [this === globalThis, name, args]; };
    `,
      context,
    )
  const cdp = {
    async send(method, params, sessionId) {
      calls.push({ method, params, sessionId })
      if (method === 'Page.addScriptToEvaluateOnNewDocument') {
        assert.equal(
          params.worldName,
          undefined,
          'fixture belongs to the main world',
        )
        assert.equal(
          params.runImmediately,
          true,
          'already loaded child frames also receive it',
        )
        for (const context of contexts) runInContext(params.source, context)
      } else if (method === 'Runtime.evaluate') {
        runInContext(params.expression, contexts[0])
      } else assert.fail(`unexpected infrastructure call ${method}`)
      return {}
    },
  }
  await installTestBuiltins(cdp, 'owned-page-session')
  assert.deepEqual(
    calls.map((call) => call.method),
    ['Page.addScriptToEvaluateOnNewDocument', 'Runtime.evaluate'],
  )
  assert.ok(calls.every((call) => call.sessionId === 'owned-page-session'))
  for (const context of contexts) {
    assert.equal(
      runInContext(
        `
      Object.entries(originals).every(([name, value]) => builtins[name] === value)
    `,
        context,
      ),
      true,
    )
    assert.equal(
      runInContext(
        `
      ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'requestAnimationFrame',
        'cancelAnimationFrame', 'requestIdleCallback', 'cancelIdleCallback'].every(name => {
        globalThis[name] = () => { throw new Error('page override must not replace captured builtin'); };
        const [bound, called, args] = builtins[name]('argument');
        return bound && called === name && args[0] === 'argument';
      })
    `,
        context,
      ),
      true,
    )
  }
  const newDocument = createContext({})
  runInContext(calls[0].params.source, newDocument)
  assert.equal(
    runInContext(
      'builtins.Date === Date && builtins.requestIdleCallback === undefined',
      newDocument,
    ),
    true,
  )
})

test('test builtins reuse the upstream clock builtins and report installation exceptions', async () => {
  const context = createContext({ __pwClock: { builtins: { marker: true } } })
  await installTestBuiltins(
    {
      async send(method, params) {
        runInContext(
          method === 'Runtime.evaluate' ? params.expression : params.source,
          context,
        )
        return {}
      },
    },
    'clock-session',
  )
  assert.equal(runInContext('builtins === __pwClock.builtins', context), true)
  await assert.rejects(
    installTestBuiltins(
      {
        async send(method) {
          return method === 'Runtime.evaluate'
            ? { exceptionDetails: { text: 'injected script threw' } }
            : {}
        },
      },
      'broken-session',
    ),
    /test builtins installation threw/,
  )
})
