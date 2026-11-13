/**
 * Failure diagnostics through the real bundled engine. Linkedom supplies the DOM and a
 * deterministic visible box; native browser tests own layout, delayed hit tests and the full
 * matcher matrix. These tests pin the output boundary and its synchronous cleanup contract.
 */
import { afterEach, expect, it, spyOn } from 'bun:test'
import { parseSelector } from '../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import { parseTemplate } from '../src/prelude/aria-template'
import type {
  ExpectParams,
  ExpectRequest,
  ProbeRequest,
  Read,
} from '../src/probe/protocol'
import { openProbePage, type ProbePage } from './support/probe-page'

const SECRET = '🔐'
const PAGE = `<html><body>
  <label id="secret-label">Password <input id="secret" type="password" value="${SECRET}"
    aria-valuetext="SpokenCanary" aria-valuenow="NumberCanary" title="Public title"></label>
  <input id="public" type="text" value="Public value">
</body></html>`

let page: ProbePage | undefined
afterEach(() => {
  page?.close()
  page = undefined
})

function booted(html = PAGE): ProbePage {
  page?.close()
  page = openProbePage(html)
  page.boot({ frameSeq: 0, epoch: 'failure-output' })
  const view = page.document.defaultView
  if (!view) throw new Error('the fixture has a window')
  const style = () => ({
    display: 'block',
    visibility: 'visible',
    cursor: 'auto',
    content: 'none',
    getPropertyValue: () => '',
  })
  Reflect.set(view, 'getComputedStyle', style)
  Reflect.set(globalThis, 'getComputedStyle', style)
  for (const element of page.document.querySelectorAll('*')) {
    Reflect.set(element, 'checkVisibility', () => true)
    Reflect.set(element, 'getBoundingClientRect', () => ({
      x: 0,
      y: 0,
      width: 10,
      height: 10,
    }))
  }
  return page
}

function element(probe: ProbePage, selector: string): Element {
  const found = probe.document.querySelector(selector)
  if (!found) throw new Error(`missing fixture element ${selector}`)
  return found
}

const target = (selector: string) => ({ chunk: parseSelector(selector) })

function expectRequest(selector: string, params: ExpectParams): ExpectRequest {
  return {
    op: 'expect',
    target: selector === 'document' ? 'document' : target(selector),
    strictness: 'strict',
    frameVisible: true,
    params,
  }
}

const check = (probe: ProbePage, selector: string, params: ExpectParams) =>
  probe.call(expectRequest(selector, params))

const read = (probe: ProbePage, selector: string, kind: Read) =>
  probe.call({
    op: 'read',
    target: target(selector),
    strictness: 'strict',
    frameVisible: true,
    read: kind,
  })

function textProperty(value: unknown, name: string): string {
  const text: unknown =
    typeof value === 'object' && value !== null
      ? Reflect.get(value, name)
      : undefined
  if (typeof text !== 'string')
    throw new Error(`no ${name} in ${JSON.stringify(value)}`)
  return text
}

/**
 * Observe the real instance when its first request installs diagnostic hooks. Keeping this
 * in the test avoids exporting the world's engine or replacing the vendor with a mock.
 */
function diagnosticEngine(probe: ProbePage): object {
  let captured: object | undefined
  const define = Object.defineProperty
  const observe = spyOn(Object, 'defineProperty').mockImplementation(
    (object, name, descriptor) => {
      if (
        name === 'previewNode' &&
        typeof object === 'object' &&
        object !== null
      )
        captured = object
      return define(object, name, descriptor)
    },
  )
  try {
    expect(probe.call({ op: 'describe' })).toMatchObject({ kind: 'described' })
  } finally {
    observe.mockRestore()
  }
  if (!captured) throw new Error('the request installed no diagnostic scope')
  return captured
}

function invoke(engine: object, name: string, args: unknown[]): unknown {
  const method: unknown = Reflect.get(engine, name)
  if (typeof method !== 'function') throw new Error(`no engine method ${name}`)
  return Reflect.apply(method, engine, args)
}

const diagnosticDescriptors = (engine: object) =>
  ['previewNode', '_generateSelectors'].map((name) =>
    Object.getOwnPropertyDescriptor(engine, name),
  )

function conformanceCall(request: ProbeRequest): unknown {
  const probe = globalThis.__neoProbe
  if (!probe) throw new Error('the fixture has no probe')
  return probe.call({
    request,
    policy: { conformance: true, mayHaveOtherFrame: true },
  })
}

it('compares real private values through labels and counts code points only in diagnostics', async () => {
  const probe = booted()
  // Linkedom omits label.control; the engine still performs its own real follow-label path.
  Reflect.set(
    element(probe, '#secret-label'),
    'control',
    element(probe, '#secret'),
  )
  for (const selector of ['#secret', '#secret-label']) {
    expect(
      await check(probe, selector, {
        expression: 'to.have.value',
        expectedText: [{ string: SECRET }],
        isNot: false,
      }),
    ).toMatchObject({
      kind: 'expect',
      matches: true,
      received: '<1 character>',
    })
    const failed = await check(probe, selector, {
      expression: 'to.have.value',
      expectedText: [{ string: 'wrong' }],
      isNot: false,
    })
    expect(failed).toMatchObject({
      kind: 'expect',
      matches: false,
      received: '<1 character>',
    })
    const encoded = JSON.stringify(failed)
    expect(encoded).not.toContain(SECRET)
    expect(encoded).not.toContain('SpokenCanary')
    expect(encoded).not.toContain('NumberCanary')
    expect(encoded).toContain('••')
  }
})

it('compares sensitive attributes before redacting and preserves null and public reports', async () => {
  const probe = booted()
  for (const { name, value, received } of [
    { name: 'value', value: SECRET, received: '<1 character>' },
    {
      name: 'aria-valuetext',
      value: 'SpokenCanary',
      received: '<12 characters>',
    },
    {
      name: 'aria-valuenow',
      value: 'NumberCanary',
      received: '<12 characters>',
    },
  ]) {
    const params = {
      expression: 'to.have.attribute.value',
      expressionArg: name,
      expectedText: [{ string: value }],
      isNot: false,
    }
    expect(await check(probe, '#secret', params)).toMatchObject({
      kind: 'expect',
      matches: true,
    })
    const failed = await check(probe, '#secret', {
      ...params,
      expectedText: [{ string: 'wrong' }],
    })
    expect(failed).toMatchObject({
      kind: 'expect',
      matches: false,
      received,
    })
    expect(JSON.stringify(failed)).not.toContain(value)
  }
  element(probe, '#secret').removeAttribute('aria-valuenow')
  expect(
    await check(probe, '#secret', {
      expression: 'to.have.attribute.value',
      expressionArg: 'aria-valuenow',
      expectedText: [{ string: 'wrong' }],
      isNot: false,
    }),
  ).toMatchObject({ kind: 'expect', matches: false, received: null })
  expect(
    await check(probe, '#public', {
      expression: 'to.have.value',
      expectedText: [{ string: 'wrong' }],
      isNot: false,
    }),
  ).toMatchObject({ kind: 'expect', matches: false, received: 'Public value' })
  expect(
    await check(probe, '#secret', {
      expression: 'to.have.attribute.value',
      expressionArg: 'title',
      expectedText: [{ string: 'wrong' }],
      isNot: false,
    }),
  ).toMatchObject({ kind: 'expect', matches: false, received: 'Public title' })
})

it('projects private textarea previews without changing the DOM or intentional reads', async () => {
  const canaries = [
    'AttributeCanary',
    'SpokenCanary',
    'NumberCanary',
    'TextCanary',
  ]
  const probe = booted(`<html><body>
    <textarea id="private" autocomplete="new-password" value="AttributeCanary"
      aria-valuetext="SpokenCanary" aria-valuenow="NumberCanary">TextCanary</textarea>
  </body></html>`)
  const field = element(probe, '#private')
  const before = field.outerHTML
  const resolved = probe.call({
    op: 'resolve',
    target: target('#private'),
    strictness: 'strict',
    want: 'element',
  })
  expect(resolved).toMatchObject({ kind: 'element' })
  const preview = textProperty(resolved, 'preview')
  expect(preview).toEndWith('></textarea>')
  for (const canary of canaries)
    expect(JSON.stringify(resolved)).not.toContain(canary)
  expect(preview).toContain('value="<15 characters>"')
  expect(preview).toContain('aria-valuetext="<12 characters>"')
  expect(preview).toContain('aria-valuenow="<12 characters>"')
  expect(field.outerHTML).toBe(before)
  expect(read(probe, '#private', { kind: 'inputValue' })).toEqual({
    kind: 'read',
    value: 'TextCanary',
  })
  expect(read(probe, '#private', { kind: 'attribute', name: 'value' })).toEqual(
    {
      kind: 'read',
      value: 'AttributeCanary',
    },
  )
  expect(read(probe, '#private', { kind: 'textContent' })).toEqual({
    kind: 'read',
    value: 'TextCanary',
  })
  const content = probe.call({ op: 'document', part: 'content' })
  for (const canary of canaries)
    expect(textProperty(content, 'text')).toContain(canary)
  expect(
    await check(probe, '#private', {
      expression: 'to.have.value',
      expectedText: [{ string: 'TextCanary' }],
      isNot: false,
    }),
  ).toMatchObject({
    kind: 'expect',
    matches: true,
    received: '<10 characters>',
  })
  expect(field.outerHTML).toBe(before)
})

it('uses guarded text for direct, ancestor and array comparisons', async () => {
  const probe = booted(`<html><body><main id="ancestor">Public prefix<textarea
    id="private" autocomplete="new-password">TextCanary</textarea></main></body></html>`)
  for (const { selector, value } of [
    { selector: '#private', value: '' },
    { selector: '#ancestor', value: 'Public prefix' },
  ]) {
    expect(
      await check(probe, selector, {
        expression: 'to.have.text',
        expectedText: [{ string: value }],
        isNot: false,
      }),
    ).toMatchObject({ kind: 'expect', matches: true, received: value })
  }
  const failed = await check(probe, '#ancestor, #private', {
    expression: 'to.have.text.array',
    expectedText: [{ string: 'wrong' }],
    isNot: false,
  })
  expect(failed).toMatchObject({
    kind: 'expect',
    matches: false,
    received: ['Public prefix', ''],
  })
  expect(JSON.stringify(failed)).not.toContain('TextCanary')
  expect(element(probe, '#private').textContent).toBe('TextCanary')
})

it('matches an aria template copied from the masked tree and guards the document fallback', async () => {
  const probe = booted()
  const fallback = probe.call({ op: 'document', part: 'expectSnapshot' })
  const yaml = textProperty(fallback, 'text')
  expect(yaml).toContain('••')
  expect(yaml).not.toContain(SECRET)
  const parsed = parseTemplate(yaml)
  if ('error' in parsed) throw new Error(parsed.error)
  expect(
    await check(probe, 'document', {
      expression: 'to.match.aria',
      expectedValue: parsed.template,
      isNot: false,
    }),
  ).toMatchObject({ kind: 'expect', matches: true })
  const clear = parseTemplate(yaml.replaceAll('••', SECRET))
  if ('error' in clear) throw new Error(clear.error)
  const failed = await check(probe, 'document', {
    expression: 'to.match.aria',
    expectedValue: clear.template,
    isNot: false,
  })
  expect(failed).toMatchObject({ kind: 'expect', matches: false })
  expect(JSON.stringify(failed)).not.toContain(SECRET)
})

it('guards strict-mode diagnostic aliases while selectors still match real textarea text', () => {
  const probe = booted(`<html><body>
    <textarea role="button" autocomplete="new-password">PrivateAliasCanary</textarea>
    <textarea role="button">Public alias</textarea>
  </body></html>`)
  const request = {
    op: 'resolve',
    target: target('textarea'),
    strictness: 'strict',
    want: 'element',
  } satisfies ProbeRequest
  const guarded = probe.call(request)
  expect(guarded).toMatchObject({ kind: 'strict' })
  const message = textProperty(guarded, 'message')
  expect(message).toContain(' aka ')
  expect(message).toContain('Public alias')
  expect(message).not.toContain('PrivateAliasCanary')
  const native = conformanceCall(request)
  expect(textProperty(native, 'message')).toContain('PrivateAliasCanary')
  expect(
    probe.call({
      ...request,
      target: target('text=PrivateAliasCanary'),
    }),
  ).toMatchObject({ kind: 'element' })
})

it('guards the same vendor aliases in deprecated-selector errors', () => {
  const probe = booted(`<html><body>
    <textarea role="button" autocomplete="new-password">PrivateAliasCanary</textarea>
    <textarea role="button">Public alias</textarea><input id="trigger" type="text">
  </body></html>`)
  const engine = diagnosticEngine(probe)
  const deprecated = () => {
    try {
      invoke(engine, 'querySelector', [
        parseSelector('text:light=/./'),
        probe.document,
        false,
      ])
    } catch (error) {
      if (!(error instanceof Error)) throw error
      return error.message
    }
    throw new Error('the deprecated selector did not throw')
  }
  expect(deprecated()).toContain('PrivateAliasCanary')
  // Neo resolves with querySelectorAll, so exercise the vendor's deprecation check from a
  // synchronous DOM read inside a real request, where the diagnostic scope must apply too.
  Object.defineProperty(element(probe, '#trigger'), 'value', {
    configurable: true,
    get: deprecated,
  })
  const message = textProperty(
    read(probe, '#trigger', { kind: 'inputValue' }),
    'value',
  )
  expect(message).toContain('"text:light" selector is not supported:')
  expect(message).toContain(' aka ')
  expect(message).toContain('Public alias')
  expect(message).not.toContain('PrivateAliasCanary')
})

it('restores exact diagnostic and DOM descriptors after a tree throw and before an await', async () => {
  const probe = booted()
  const engine = diagnosticEngine(probe)
  const nativePreview: unknown = Reflect.get(engine, 'previewNode')
  const nativeSelectors: unknown = Reflect.get(engine, '_generateSelectors')
  Object.defineProperty(engine, 'previewNode', {
    configurable: true,
    enumerable: true,
    get: () => nativePreview,
  })
  Object.defineProperty(engine, '_generateSelectors', {
    configurable: true,
    enumerable: true,
    writable: false,
    value: nativeSelectors,
  })
  const diagnostics = diagnosticDescriptors(engine)
  const guardedProperties: [object, string][] = [
    [HTMLInputElement.prototype, 'value'],
    [HTMLInputElement.prototype, 'getAttribute'],
    [HTMLTextAreaElement.prototype, 'textContent'],
    [Text.prototype, 'nodeValue'],
    [globalThis, 'getComputedStyle'],
  ]
  const descriptors = () =>
    guardedProperties.map(([object, name]) =>
      Object.getOwnPropertyDescriptor(object, name),
    )
  const dom = descriptors()
  const secret = element(probe, '#secret')
  const getAttribute = secret.getAttribute.bind(secret)
  secret.getAttribute = (name: string) => {
    if (name === 'title') throw new Error('tree walk failed')
    return getAttribute(name)
  }
  try {
    expect(
      probe.call({ op: 'document', part: 'expectSnapshot' }),
    ).toMatchObject({
      kind: 'error',
      message: 'tree walk failed',
    })
  } finally {
    secret.getAttribute = getAttribute
  }
  expect(diagnosticDescriptors(engine)).toEqual(diagnostics)
  expect(descriptors()).toEqual(dom)
  const pending = check(probe, '#secret', {
    expression: 'to.have.accessible.name',
    expectedText: [{ string: 'wrong' }],
    isNot: false,
  })
  expect(pending).toBeInstanceOf(Promise)
  expect(diagnosticDescriptors(engine)).toEqual(diagnostics)
  expect(descriptors()).toEqual(dom)
  expect(read(probe, '#secret', { kind: 'inputValue' })).toEqual({
    kind: 'read',
    value: SECRET,
  })
  expect(await pending).toMatchObject({ kind: 'expect', matches: false })
  expect(diagnosticDescriptors(engine)).toEqual(diagnostics)
  expect(descriptors()).toEqual(dom)
})

it('restores an outer request diagnostic scope after a nested request', () => {
  const probe = booted()
  const engine = diagnosticEngine(probe)
  const before = diagnosticDescriptors(engine)
  const field = element(probe, '#secret')
  const getAttribute = field.getAttribute.bind(field)
  let nested = false
  field.getAttribute = (name: string) => {
    if (!nested && name === 'type') {
      nested = true
      const outer = diagnosticDescriptors(engine)
      expect(probe.call({ op: 'describe' })).toMatchObject({
        kind: 'described',
      })
      expect(diagnosticDescriptors(engine)).toEqual(outer)
    }
    return getAttribute(name)
  }
  const resolved = probe.call({
    op: 'resolve',
    target: target('#secret'),
    strictness: 'strict',
    want: 'element',
  })
  expect(nested).toBe(true)
  expect(resolved).toMatchObject({ kind: 'element' })
  expect(textProperty(resolved, 'preview')).toContain('value="<1 character>"')
  expect(diagnosticDescriptors(engine)).toEqual(before)
})

it('conformance installs no diagnostic, value, text or style hooks and keeps native reports', async () => {
  const probe = booted()
  const engine = diagnosticEngine(probe)
  const params: ExpectParams = {
    expression: 'to.have.value',
    expectedText: [{ string: 'wrong' }],
    isNot: false,
  }
  const field = element(probe, '#secret')
  const nativePreview = invoke(engine, 'previewNode', [field])
  const nativeCheck = await invoke(engine, 'expectSingleElement', [
    field,
    params,
    true,
  ])
  const nativeTree = invoke(engine, '_ariaSnapshotForExpect', [
    field,
    params,
    true,
  ])
  const replacements: unknown[] = []
  const define = Object.defineProperty
  const observe = spyOn(Object, 'defineProperty').mockImplementation(
    (object, name, descriptor) => {
      if (
        object === engine ||
        object === HTMLInputElement.prototype ||
        object === HTMLTextAreaElement.prototype ||
        object === Text.prototype ||
        (object === globalThis && name === 'getComputedStyle')
      )
        replacements.push({ object, name })
      return define(object, name, descriptor)
    },
  )
  try {
    const result = await conformanceCall(expectRequest('#secret', params))
    expect(nativeCheck).toEqual({ matches: false, received: SECRET })
    expect(result).toMatchObject({
      kind: 'expect',
      matches: false,
      received: SECRET,
      resolved: nativePreview,
      ariaSnapshot: nativeTree,
    })
    expect(JSON.stringify(result)).toContain(SECRET)
    expect(JSON.stringify(result)).toContain('SpokenCanary')
    expect(replacements).toEqual([])
  } finally {
    observe.mockRestore()
  }
})

it('keeps a host-typed mark protected in conformance while declared fields stay native', async () => {
  const probe = booted()
  const field = element(probe, '#public')
  field.setAttribute('value', 'MarkedCanary')
  const resolved = probe.call({
    op: 'resolve',
    target: { chunk: parseSelector('#public') },
    strictness: 'strict',
    want: 'element',
  })
  if (
    typeof resolved !== 'object' ||
    resolved === null ||
    !('pin' in resolved) ||
    typeof resolved.pin !== 'number'
  )
    throw new Error('the public field was pinned for private entry')
  const pin = resolved.pin
  await probe.call({
    op: 'private',
    pin,
    step: { kind: 'focus', text: 'MarkedCanary', spaced: false },
  })
  probe.call({ op: 'private', pin, step: { kind: 'forget' } })
  const params: ExpectParams = {
    expression: 'to.have.value',
    expectedText: [{ string: 'wrong' }],
    isNot: false,
  }
  const marked = await conformanceCall(expectRequest('#public', params))
  expect(marked).toMatchObject({
    kind: 'expect',
    matches: false,
    received: '••••••••••••',
  })
  expect(JSON.stringify(marked)).not.toContain('MarkedCanary')
  expect(textProperty(marked, 'resolved')).toContain('value="<12 characters>"')
  const declared = await conformanceCall(expectRequest('#secret', params))
  expect(declared).toMatchObject({ received: SECRET })
  expect(JSON.stringify(declared)).toContain('SpokenCanary')
  expect(field.getAttribute('value')).toBe('MarkedCanary')
})

it('guards accessible error-message comparisons for declared and host-marked sources', async () => {
  const canary = 'ErrCanary-4'
  const masked = `Wrong: ${'•'.repeat(canary.length)}`
  // Linkedom has no CSS.escape. Every id in this fixture is a plain identifier; native
  // browser rows cover the actual ARIA id-reference resolver and CSS escaping.
  const css = Object.getOwnPropertyDescriptor(globalThis, 'CSS')
  Object.defineProperty(globalThis, 'CSS', {
    configurable: true,
    value: { escape: (id: string) => id },
  })
  try {
    for (const marked of [false, true]) {
      const probe = booted(`<html><body>
      <input id="source" type="${marked ? 'text' : 'password'}" value="${canary}">
      <input id="invalid" type="text" aria-invalid="true" aria-errormessage="message">
      <div id="message">Wrong: <button aria-labelledby="source">Error source</button></div>
    </body></html>`)
      const params: ExpectParams = {
        expression: 'to.have.accessible.error.message',
        expectedText: [{ string: 'wrong' }],
        isNot: false,
      }
      expect(
        await conformanceCall(expectRequest('#invalid', params)),
      ).toMatchObject({ received: `Wrong: ${canary}` })
      if (marked) {
        const resolved = probe.call({
          op: 'resolve',
          target: target('#source'),
          strictness: 'strict',
          want: 'element',
        })
        if (
          typeof resolved !== 'object' ||
          resolved === null ||
          !('pin' in resolved) ||
          typeof resolved.pin !== 'number'
        )
          throw new Error('the source was pinned for private entry')
        await probe.call({
          op: 'private',
          pin: resolved.pin,
          step: { kind: 'focus', text: canary, spaced: false },
        })
        probe.call({
          op: 'private',
          pin: resolved.pin,
          step: { kind: 'forget' },
        })
      }
      const failed = await check(probe, '#invalid', params)
      expect(failed).toMatchObject({ matches: false, received: masked })
      expect(JSON.stringify(failed)).not.toContain(canary)
      const passing = { ...params, expectedText: [{ string: masked }] }
      expect(await check(probe, '#invalid', passing)).toMatchObject({
        matches: true,
        received: masked,
      })
      if (marked)
        expect(
          await conformanceCall(expectRequest('#invalid', passing)),
        ).toMatchObject({ matches: true, received: masked })
      expect(element(probe, '#source').getAttribute('value')).toBe(canary)
    }
  } finally {
    if (css) Object.defineProperty(globalThis, 'CSS', css)
    else Reflect.deleteProperty(globalThis, 'CSS')
  }
})
