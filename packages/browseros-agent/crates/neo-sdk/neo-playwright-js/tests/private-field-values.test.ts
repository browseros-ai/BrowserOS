import { afterEach, beforeEach, expect, it, spyOn } from 'bun:test'
import { asPointerTargets } from '../src/probe/pointer-targets'
import { privateStep } from '../src/probe/private-entry'
import {
  withMarkedFieldsMasked,
  withMaskedFieldValues,
} from '../src/probe/private-field-values'
import type { World } from '../src/probe/world'
import { openProbePage, type ProbePage } from './support/probe-page'

let page: ProbePage
beforeEach(() => {
  page = openProbePage('<html><body></body></html>')
})
afterEach(() => page.close())

/** Existing local-realm cases keep the ordinary no-neighbour snapshot path. */
function masked<T>(walk: () => T): T {
  return withMaskedFieldValues(
    { root: page.document.body, mayHaveOtherFrame: false },
    walk,
  )
}

function field(
  tag: 'input' | 'textarea',
  attributes: Record<string, string>,
  value: string,
) {
  const element = page.document.createElement(tag)
  for (const [name, content] of Object.entries(attributes))
    element.setAttribute(name, content)
  element.value = value
  return element
}

/**
 * linkedom shares constructors across documents. Sibling prototypes with native descriptors
 * model retained wrapper identities without inheriting the local control/Text guards. Actual
 * cross-frame adoption and isolated-world separation belong to the native integration tests.
 */
function siblingPrototype(prototype: object): object {
  const sibling: object = Object.create(Object.getPrototypeOf(prototype))
  return Object.defineProperties(
    sibling,
    Object.getOwnPropertyDescriptors(prototype),
  )
}

function retainedField(
  tag: 'input' | 'textarea',
  attributes: Record<string, string>,
  value: string,
) {
  const element = field(tag, attributes, value)
  const prototype = siblingPrototype(Object.getPrototypeOf(element))
  Object.setPrototypeOf(element, prototype)
  return { element, prototype }
}

function withOpener<T>(descriptor: PropertyDescriptor, walk: () => T): T {
  const view = page.document.defaultView
  if (!view) throw new Error('the fixture supplies a window')
  const original = Object.getOwnPropertyDescriptor(view, 'opener')
  Object.defineProperty(view, 'opener', {
    configurable: true,
    ...descriptor,
  })
  try {
    return walk()
  } finally {
    if (original) Object.defineProperty(view, 'opener', original)
    else Reflect.deleteProperty(view, 'opener')
  }
}

const descriptors = () => ({
  input: Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value'),
  inputDefault: Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'defaultValue',
  ),
  textarea: Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'value',
  ),
  inputAttribute: Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'getAttribute',
  ),
  textareaAttribute: Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'getAttribute',
  ),
  textNodeValue: Object.getOwnPropertyDescriptor(Text.prototype, 'nodeValue'),
  textContent: Object.getOwnPropertyDescriptor(Text.prototype, 'textContent'),
  textData: Object.getOwnPropertyDescriptor(Text.prototype, 'data'),
  wholeText: Object.getOwnPropertyDescriptor(Text.prototype, 'wholeText'),
  textareaContent: Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'textContent',
  ),
  textareaInnerText: Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'innerText',
  ),
  textareaInnerHTML: Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'innerHTML',
  ),
  textareaDefault: Object.getOwnPropertyDescriptor(
    HTMLTextAreaElement.prototype,
    'defaultValue',
  ),
  elementAttribute: Object.getOwnPropertyDescriptor(
    Element.prototype,
    'getAttribute',
  ),
  nodeValue: Object.getOwnPropertyDescriptor(Node.prototype, 'nodeValue'),
  nodeContent: Object.getOwnPropertyDescriptor(Node.prototype, 'textContent'),
  computedStyle: Object.getOwnPropertyDescriptor(
    globalThis,
    'getComputedStyle',
  ),
})

function inheritedDescriptor(prototype: object, property: string) {
  let owner: object | null = prototype
  while (owner) {
    const descriptor = Object.getOwnPropertyDescriptor(owner, property)
    if (descriptor) return descriptor
    owner = Object.getPrototypeOf(owner)
  }
  throw new Error(`the fixture supplies ${property}`)
}

function overrideProperties(): [object, string][] {
  return [
    [HTMLInputElement.prototype, 'value'],
    [HTMLTextAreaElement.prototype, 'value'],
    [HTMLInputElement.prototype, 'defaultValue'],
    [HTMLInputElement.prototype, 'getAttribute'],
    [HTMLTextAreaElement.prototype, 'getAttribute'],
    ...['nodeValue', 'textContent', 'data', 'wholeText'].map(
      (property): [object, string] => [Text.prototype, property],
    ),
    ...['textContent', 'innerText', 'innerHTML', 'defaultValue'].map(
      (property): [object, string] => [HTMLTextAreaElement.prototype, property],
    ),
    [globalThis, 'getComputedStyle'],
  ]
}

it('masks only whole page-declared private tokens, case insensitively and across whitespace', () => {
  const cases: Record<string, string>[] = [
    { type: 'password' },
    { type: 'PaSsWoRd' },
    { autocomplete: 'current-password' },
    { autocomplete: 'new-password' },
    { autocomplete: 'cc-csc' },
    { autocomplete: 'one-time-code' },
    { autocomplete: 'CC-NUMBER' },
    { autocomplete: '  section-pay\tshipping\ncc-number\r ' },
  ]
  for (const tag of ['input', 'textarea'] as const) {
    for (const attributes of cases) {
      const element = field(tag, attributes, 'A🔒z')
      expect(masked(() => element.value)).toBe('••••')
      expect(element.value).toBe('A🔒z')
    }
  }
})

it('masks a box Neo typed a host value into, whatever the page declares (ruling R14)', async () => {
  const typed = field(
    'input',
    { type: 'text', autocomplete: 'username' },
    'A🔒z',
  )
  const ordinary = field('input', { type: 'text' }, 'plain')
  page.document.body.append(typed, ordinary)
  // The entry's focus step is what marks a box; this world reaches only what that step reads.
  const world = {
    pins: { live: (pin: number) => (pin === 7 ? typed : undefined) },
    injected: { selectText: () => 'selected' },
  } as unknown as World
  await privateStep(world, {
    op: 'private',
    pin: 7,
    step: { kind: 'focus', text: 'A🔒z', spaced: false },
  })
  privateStep(world, { op: 'private', pin: 7, step: { kind: 'forget' } })
  expect(withMaskedFieldValues(() => typed.value)).toBe('••••')
  expect(withMaskedFieldValues(() => ordinary.value)).toBe('plain')
  expect(typed.value).toBe('A🔒z')
})

it('masks a box Neo typed into under the conformance switch, where declared fields print as upstream (ruling R44)', async () => {
  const typed = field('input', { type: 'text' }, 'A🔒z')
  const declared = field('input', { type: 'password' }, 'hunter2')
  page.document.body.append(typed, declared)
  const world = {
    pins: { live: (pin: number) => (pin === 8 ? typed : undefined) },
    injected: { selectText: () => 'selected' },
  } as unknown as World
  await privateStep(world, {
    op: 'private',
    pin: 8,
    step: { kind: 'focus', text: 'A🔒z', spaced: false },
  })
  privateStep(world, { op: 'private', pin: 8, step: { kind: 'forget' } })
  // The reads outside main's masking (a failed expect's snapshot) mask the mark alone.
  expect(withMarkedFieldsMasked(() => typed.value)).toBe('••••')
  expect(withMarkedFieldsMasked(() => declared.value)).toBe('hunter2')
  const conformance = { declared: false }
  expect(withMaskedFieldValues(() => typed.value, conformance)).toBe('••••')
  expect(withMaskedFieldValues(() => declared.value, conformance)).toBe(
    'hunter2',
  )
  expect(withMaskedFieldValues(() => declared.value)).toBe('•••••••')
})

it('keeps empty secrets and ordinary fields, submit labels and expiry selects unchanged', () => {
  const empty = field('input', { type: 'password' }, '')
  const ordinaryAttributes: Record<string, string>[] = [
    {},
    { autocomplete: 'off' },
    { autocomplete: 'cc-numberx' },
    { autocomplete: 'xcurrent-password' },
    { autocomplete: 'cc-exp' },
    { autocomplete: 'cc-exp-month' },
    { autocomplete: 'cc-exp-year' },
    { autocomplete: 'cc-name' },
    { type: 'submit' },
    { type: 'reset' },
  ]
  const ordinary = ordinaryAttributes.map((attributes) =>
    field('input', attributes, '4111111111111111'),
  )
  const select = page.document.createElement('select')
  select.setAttribute('autocomplete', 'cc-exp-month')
  select.innerHTML = '<option value="12" selected>December</option>'
  expect(
    masked(() => [
      empty.value,
      ...ordinary.map((input) => input.value),
      select.value,
    ]),
  ).toEqual(['', ...ordinary.map(() => '4111111111111111'), '12'])
})

it('masks control value attributes while leaving other attributes and ordinary elements alone', () => {
  for (const tag of ['input', 'textarea'] as const) {
    const secret = field(tag, { autocomplete: 'new-password' }, 'typed')
    const ordinary = field(tag, {}, 'typed')
    for (const name of ['value', 'aria-valuetext', 'aria-valuenow']) {
      secret.setAttribute(name, 'A🔒z')
      ordinary.setAttribute(name, 'A🔒z')
      expect(masked(() => secret.getAttribute(name))).toBe('••••')
      expect(masked(() => ordinary.getAttribute(name))).toBe('A🔒z')
      expect(secret.getAttribute(name)).toBe('A🔒z')
    }
    expect(
      masked(() => [
        secret.getAttribute('autocomplete'),
        secret.getAttribute('missing'),
      ]),
    ).toEqual(['new-password', null])
  }
  const element = page.document.createElement('div')
  element.setAttribute('autocomplete', 'new-password')
  element.setAttribute('value', 'ordinary-div')
  const original = descriptors().elementAttribute
  masked(() => {
    expect(element.getAttribute('value')).toBe('ordinary-div')
    expect(descriptors().elementAttribute).toEqual(original)
  })
})

it('suppresses secret textarea text reads without suppressing its masked value or ordinary text', () => {
  const secret = field('textarea', { autocomplete: 'new-password' }, 'A🔒z')
  const ordinary = field('textarea', {}, 'ordinary-text')
  const input = field('input', { type: 'password' }, 'private-input')
  input.textContent = 'input-text'
  const text = secret.firstChild
  const ordinaryText = ordinary.firstChild
  if (!(text instanceof Text) || !(ordinaryText instanceof Text))
    throw new Error('textareas have text nodes')
  expect(
    masked(() => ({
      value: secret.value,
      textareaText: [
        secret.textContent,
        text.nodeValue,
        text.textContent,
        text.data,
        text.wholeText,
        secret.innerText,
        secret.innerHTML,
        secret.defaultValue,
      ],
      inputDefault: input.defaultValue,
      ordinary: [
        ordinary.value,
        ordinary.textContent,
        ordinaryText.nodeValue,
        ordinaryText.textContent,
      ],
      inputContent: input.textContent,
    })),
  ).toEqual({
    value: '••••',
    textareaText: Array(8).fill(''),
    inputDefault: '•••••••••••••',
    ordinary: Array(4).fill('ordinary-text'),
    inputContent: 'input-text',
  })
  expect([
    secret.value,
    secret.textContent,
    text.nodeValue,
    text.textContent,
  ]).toEqual(['A🔒z', 'A🔒z', 'A🔒z', 'A🔒z'])
})

it('suppresses both CSS content reads for private controls and their pseudo-elements', () => {
  const nativeStyle = page.document.createElement('div').style
  nativeStyle.content = '"CSSSecretCanary"'
  nativeStyle.cursor = 'text'
  nativeStyle.display = 'inline-block'
  const other = page.document.createElement('div')
  other.setAttribute('autocomplete', 'one-time-code')
  const attributes = spyOn(other, 'getAttribute')
  const read = spyOn(globalThis, 'getComputedStyle').mockReturnValue(
    nativeStyle,
  )
  try {
    for (const tag of ['input', 'textarea'] as const) {
      const secret = field(tag, { autocomplete: 'one-time-code' }, 'private')
      const ordinary = field(tag, {}, 'public')
      masked(() => {
        for (const pseudo of [
          undefined,
          null,
          '',
          '::before',
          '::after',
          ':before',
          '::marker',
        ]) {
          const style = getComputedStyle(secret, pseudo)
          expect(style.content).toBe('none')
          expect(style.getPropertyValue('content')).toBe('none')
          expect(style.cursor).toBe('text')
          expect(style.display).toBe('inline-block')
          expect(style.getPropertyValue('display')).toBe('inline-block')
          expect(getComputedStyle(ordinary, pseudo)).toBe(nativeStyle)
          expect(getComputedStyle(other, pseudo)).toBe(nativeStyle)
        }
        expect(attributes).not.toHaveBeenCalled()
      })
      expect(getComputedStyle(secret)).toBe(nativeStyle)
      expect(getComputedStyle(secret, '::before')).toBe(nativeStyle)
      expect(nativeStyle.content).toBe('"CSSSecretCanary"')
    }
  } finally {
    read.mockRestore()
    attributes.mockRestore()
  }
})

it('composes with pointer marking in either nesting order and restores each enclosing style reader', () => {
  const secret = field('input', { autocomplete: 'cc-number' }, 'private')
  const nativeStyle = page.document.createElement('div').style
  nativeStyle.content = '"CSSSecretCanary"'
  nativeStyle.cursor = 'text'
  const read = spyOn(globalThis, 'getComputedStyle').mockReturnValue(
    nativeStyle,
  )
  const marked = <T>(walk: () => T) => asPointerTargets([secret], walk)
  const assertCombinedReads = () => {
    for (const pseudo of [undefined, null, '', '::before', '::after']) {
      const style = getComputedStyle(secret, pseudo)
      expect(style.content).toBe('none')
      expect(style.getPropertyValue('content')).toBe('none')
      expect(style.cursor).toBe(pseudo ? 'text' : 'pointer')
    }
  }
  try {
    const before = descriptors()
    for (const [outer, inner] of [
      [masked, marked],
      [marked, masked],
    ] as const) {
      outer(() => {
        const enclosing = descriptors()
        inner(assertCombinedReads)
        expect(descriptors()).toEqual(enclosing)
        expect(() =>
          inner(() => {
            assertCombinedReads()
            throw new Error('combined walk failed')
          }),
        ).toThrow('combined walk failed')
        expect(descriptors()).toEqual(enclosing)
      })
      expect(descriptors()).toEqual(before)
      expect(getComputedStyle(secret)).toBe(nativeStyle)
      expect(getComputedStyle(secret, '::before')).toBe(nativeStyle)
    }
  } finally {
    read.mockRestore()
  }
})

it('preserves setters and descriptor flags, restoring all own descriptors after success', () => {
  const input = field('input', { type: 'password' }, 'old')
  const textarea = field('textarea', { autocomplete: 'new-password' }, 'old')
  const before = descriptors()
  const originals = overrideProperties().map(([prototype, property]) => ({
    prototype,
    property,
    descriptor: inheritedDescriptor(prototype, property),
  }))
  expect(
    masked(() => {
      const during = descriptors()
      for (const { prototype, property, descriptor } of originals) {
        const masked = Object.getOwnPropertyDescriptor(prototype, property)
        if (descriptor.get) {
          expect(masked).toEqual({ ...descriptor, get: expect.any(Function) })
          expect(masked?.get).not.toBe(descriptor.get)
        } else {
          expect(masked).toEqual({ ...descriptor, value: expect.any(Function) })
          expect(masked?.value).not.toBe(descriptor.value)
        }
      }
      expect(during.elementAttribute).toEqual(before.elementAttribute)
      expect(during.nodeValue).toEqual(before.nodeValue)
      expect(during.nodeContent).toEqual(before.nodeContent)
      input.value = 'next'
      textarea.value = 'other'
      textarea.textContent = 'other'
      const text = textarea.firstChild
      if (!text) throw new Error('the textarea has text')
      text.nodeValue = 'other'
      text.textContent = 'other'
      return [input.value, textarea.value]
    }),
  ).toEqual(['••••', '•••••'])
  expect(descriptors()).toEqual(before)
  expect([input.value, textarea.value]).toEqual(['next', 'other'])
})

it('restores all shadows when the walk throws and preserves nested synchronous scopes', () => {
  const input = field('input', { type: 'password' }, 'secret')
  const before = descriptors()
  expect(() =>
    masked(() => {
      expect(input.value).toBe('••••••')
      const outer = descriptors()
      expect(masked(() => input.value)).toBe('••••••')
      expect(descriptors()).toEqual(outer)
      expect(input.value).toBe('••••••')
      throw new Error('walk failed')
    }),
  ).toThrow('walk failed')
  expect(descriptors()).toEqual(before)
  expect(input.value).toBe('secret')
})

it('restores every earlier patch and never walks after any of the fourteen installations fails', () => {
  const before = descriptors()
  const define = Object.defineProperty
  for (const [prototype, property] of overrideProperties()) {
    const install = spyOn(Object, 'defineProperty').mockImplementation(
      (object, name, descriptor) => {
        if (object === prototype && name === property)
          throw new Error('installation refused')
        return define(object, name, descriptor)
      },
    )
    let walked = false
    try {
      expect(() =>
        masked(() => {
          walked = true
        }),
      ).toThrow('installation refused')
      expect(walked).toBe(false)
      expect(descriptors()).toEqual(before)
    } finally {
      install.mockRestore()
    }
  }
})

it('validates both value accessors before installing either override', () => {
  const before = descriptors()
  if (!before.textarea) throw new Error('the textarea getter exists')
  Object.defineProperty(HTMLTextAreaElement.prototype, 'value', {
    ...before.textarea,
    get: undefined,
  })
  try {
    expect(() => masked(() => 'unreachable')).toThrow(
      'snapshot field value accessor is unavailable',
    )
    expect(descriptors().input).toEqual(before.input)
  } finally {
    Object.defineProperty(
      HTMLTextAreaElement.prototype,
      'value',
      before.textarea,
    )
  }
})

it('validates the last accessor before installing any override', () => {
  const before = descriptors()
  Object.defineProperty(HTMLTextAreaElement.prototype, 'defaultValue', {
    configurable: true,
    get: undefined,
  })
  const define = spyOn(Object, 'defineProperty')
  try {
    expect(() => masked(() => 'unreachable')).toThrow(
      'snapshot textarea text accessor is unavailable',
    )
    expect(define).not.toHaveBeenCalled()
  } finally {
    define.mockRestore()
    if (before.textareaDefault)
      Object.defineProperty(
        HTMLTextAreaElement.prototype,
        'defaultValue',
        before.textareaDefault,
      )
    else Reflect.deleteProperty(HTMLTextAreaElement.prototype, 'defaultValue')
  }
  expect(descriptors()).toEqual(before)
})

it('validates the style reader before installing any override', () => {
  const before = descriptors()
  if (!before.computedStyle) throw new Error('the style reader exists')
  Object.defineProperty(globalThis, 'getComputedStyle', {
    ...before.computedStyle,
    value: undefined,
  })
  const define = spyOn(Object, 'defineProperty')
  try {
    expect(() => masked(() => 'unreachable')).toThrow(
      'snapshot getComputedStyle method is unavailable',
    )
    expect(define).not.toHaveBeenCalled()
  } finally {
    define.mockRestore()
    Object.defineProperty(globalThis, 'getComputedStyle', before.computedStyle)
  }
  expect(descriptors()).toEqual(before)
})

it('attempts every restoration in reverse even when a cleanup reports failure', () => {
  const before = descriptors()
  const define = Object.defineProperty
  const remove = Reflect.deleteProperty
  const restored: string[] = []
  let cleaning = false
  const install = spyOn(Object, 'defineProperty').mockImplementation(
    (object, name, descriptor) => {
      const result = define(object, name, descriptor)
      if (cleaning) {
        restored.push(String(name))
        if (
          (object === HTMLTextAreaElement.prototype && name === 'value') ||
          (object === globalThis && name === 'getComputedStyle')
        )
          throw new Error('restoration reported failure')
      }
      return result
    },
  )
  const deletion = spyOn(Reflect, 'deleteProperty').mockImplementation(
    (object, name) => {
      const result = remove(object, name)
      if (cleaning) {
        restored.push(String(name))
        if (object === Text.prototype && name === 'textContent')
          throw new Error('shadow removal reported failure')
      }
      return result
    },
  )
  try {
    expect(() =>
      masked(() => {
        cleaning = true
        throw new Error('walk failed')
      }),
    ).toThrow('snapshot field overrides could not be restored')
    expect(restored).toEqual([
      'getComputedStyle',
      'defaultValue',
      'innerHTML',
      'innerText',
      'textContent',
      'wholeText',
      'data',
      'textContent',
      'nodeValue',
      'getAttribute',
      'getAttribute',
      'defaultValue',
      'value',
      'value',
    ])
    expect(descriptors()).toEqual(before)
  } finally {
    install.mockRestore()
    deletion.mockRestore()
  }
})

it('discovers an external retained input from the scoped root owner document', () => {
  const { element, prototype } = retainedField(
    'input',
    { type: 'password' },
    'A🔒z',
  )
  element.setAttribute('id', 'external-value')
  element.setAttribute('aria-valuetext', 'A🔒z')
  element.setAttribute('aria-valuenow', 'A🔒z')
  const button = page.document.createElement('button')
  button.setAttribute('aria-labelledby', 'external-value')
  page.document.body.append(button, element)
  expect(element instanceof HTMLInputElement).toBe(false)
  const before = Object.getOwnPropertyDescriptors(prototype)
  const query = spyOn(page.document, 'querySelectorAll')
  const scopedQuery = spyOn(button, 'querySelectorAll')
  try {
    expect(
      withMaskedFieldValues({ root: button, mayHaveOtherFrame: true }, () => [
        element.value,
        element.defaultValue,
        element.getAttribute('value'),
        element.getAttribute('aria-valuetext'),
        element.getAttribute('aria-valuenow'),
      ]),
    ).toEqual(Array(5).fill('••••'))
    expect(query.mock.calls).toEqual([['input,textarea'], ['*']])
    expect(scopedQuery).not.toHaveBeenCalled()
    expect(element.ownerDocument).toBe(page.document)
    expect(element.value).toBe('A🔒z')
    expect(Object.getOwnPropertyDescriptors(prototype)).toEqual(before)
  } finally {
    query.mockRestore()
    scopedQuery.mockRestore()
  }
})

it('always collects a retained field root while document discovery is skipped', () => {
  withOpener({ value: null }, () => {
    const query = spyOn(page.document, 'querySelectorAll')
    try {
      for (const tag of ['input', 'textarea'] as const) {
        const { element, prototype } = retainedField(
          tag,
          { autocomplete: 'one-time-code' },
          'RootSecret',
        )
        const before = Object.getOwnPropertyDescriptors(prototype)
        expect(
          withMaskedFieldValues(
            { root: element, mayHaveOtherFrame: false },
            () => element.value,
          ),
        ).toBe('••••••••••')
        expect(element.value).toBe('RootSecret')
        expect(Object.getOwnPropertyDescriptors(prototype)).toEqual(before)
      }
      expect(query).not.toHaveBeenCalled()
    } finally {
      query.mockRestore()
    }
  })
})

for (const retainedTextarea of [false, true]) {
  it(`guards a ${retainedTextarea ? 'retained' : 'local'} textarea and every independently retained direct Text prototype`, () => {
    const element = field('textarea', { autocomplete: 'new-password' }, '')
    const prototype = retainedTextarea
      ? siblingPrototype(HTMLTextAreaElement.prototype)
      : HTMLTextAreaElement.prototype
    Object.setPrototypeOf(element, prototype)
    const first = page.document.createTextNode('A')
    const second = page.document.createTextNode('🔒')
    const local = page.document.createTextNode('z')
    const firstPrototype = siblingPrototype(Text.prototype)
    const secondPrototype = siblingPrototype(Text.prototype)
    Object.setPrototypeOf(first, firstPrototype)
    Object.setPrototypeOf(second, secondPrototype)
    element.replaceChildren(first, second, local)
    page.document.body.append(element)
    expect(element instanceof HTMLTextAreaElement).toBe(!retainedTextarea)
    expect(first instanceof Text).toBe(false)
    expect(second instanceof Text).toBe(false)
    const targets = [prototype, firstPrototype, secondPrototype]
    const before = targets.map((target) =>
      Object.getOwnPropertyDescriptors(target),
    )
    expect(
      withMaskedFieldValues(
        { root: page.document.body, mayHaveOtherFrame: true },
        () => ({
          value: element.value,
          ownText: [
            element.textContent,
            element.innerText,
            element.innerHTML,
            element.defaultValue,
          ],
          childText: [first, second, local].flatMap((text) => [
            text.nodeValue,
            text.textContent,
            text.data,
            text.wholeText,
          ]),
        }),
      ),
    ).toEqual({
      value: '••••',
      ownText: Array(4).fill(''),
      childText: Array(12).fill(''),
    })
    expect(element.value).toBe('A🔒z')
    expect([first.data, second.data, local.data]).toEqual(['A', '🔒', 'z'])
    expect(
      targets.map((target) => Object.getOwnPropertyDescriptors(target)),
    ).toEqual(before)
  })
}

it('skips document and shadow searches on a large page without a neighbour or opener', () => {
  page.document.body.innerHTML = '<div>ordinary</div>'.repeat(10_000)
  const host = page.document.createElement('section')
  const shadow = host.attachShadow({ mode: 'open' })
  const secret = field('input', { type: 'password' }, 'local')
  shadow.append(secret)
  page.document.body.append(host)
  withOpener({ value: null }, () => {
    const queries = [
      spyOn(page.document, 'querySelectorAll'),
      spyOn(page.document.body, 'querySelectorAll'),
      spyOn(shadow, 'querySelectorAll'),
    ]
    try {
      expect(
        withMaskedFieldValues(
          { root: page.document.body, mayHaveOtherFrame: false },
          () => secret.value,
        ),
      ).toBe('•••••')
      for (const query of queries) expect(query).not.toHaveBeenCalled()
      expect(secret.value).toBe('local')
    } finally {
      for (const query of queries) query.mockRestore()
    }
  })
})

it('recurses through every open and exposed closed shadow root when the hint is true', () => {
  const host = page.document.createElement('section')
  const open = host.attachShadow({ mode: 'open' })
  const closedHost = page.document.createElement('section')
  const closed = closedHost.attachShadow({ mode: 'closed' })
  // The production shadow getter exposes handed closed roots before masking. Model its
  // answer here so this test does not depend on another module's persistent installation.
  Object.defineProperty(closedHost, 'shadowRoot', { get: () => closed })
  const nestedHost = page.document.createElement('section')
  const nested = nestedHost.attachShadow({ mode: 'open' })
  page.document.body.append(host)
  open.append(closedHost)
  closed.append(nestedHost)
  const scopes = [page.document, open, closed, nested]
  const secrets = scopes.map((scope, index) => {
    const { element } = retainedField(
      index % 2 ? 'textarea' : 'input',
      { autocomplete: 'cc-number' },
      'secret',
    )
    if (scope === page.document) page.document.body.append(element)
    else scope.append(element)
    return element
  })
  const queries = scopes.map((scope) => spyOn(scope, 'querySelectorAll'))
  try {
    expect(
      withMaskedFieldValues({ root: host, mayHaveOtherFrame: true }, () =>
        secrets.map((secret) => secret.value),
      ),
    ).toEqual(Array(4).fill('••••••'))
    for (const query of queries)
      expect(query.mock.calls).toEqual([['input,textarea'], ['*']])
    expect(secrets.map((secret) => secret.value)).toEqual(
      Array(4).fill('secret'),
    )
  } finally {
    for (const query of queries) query.mockRestore()
  }
})

it('enables discovery for a SOP-readable opener without consulting origin strings', () => {
  const { element } = retainedField('input', { type: 'password' }, 'secret')
  page.document.body.append(element)
  let documentReads = 0
  const opener = {
    get document() {
      documentReads++
      return page.document
    },
    get location() {
      throw new Error('URL strings do not establish SOP access')
    },
  }
  withOpener({ value: opener }, () => {
    const query = spyOn(page.document, 'querySelectorAll')
    try {
      expect(
        withMaskedFieldValues(
          { root: page.document.body, mayHaveOtherFrame: false },
          () => element.value,
        ),
      ).toBe('••••••')
      expect(documentReads).toBe(1)
      expect(query.mock.calls).toEqual([['input,textarea'], ['*']])
    } finally {
      query.mockRestore()
    }
  })
})

it('skips discovery for a SOP-denied opener and retains the explicit field-root guard', () => {
  const { element } = retainedField('input', { type: 'password' }, 'secret')
  const opener = {
    get document() {
      throw new DOMException('cross-origin access denied', 'SecurityError')
    },
  }
  withOpener({ value: opener }, () => {
    const query = spyOn(page.document, 'querySelectorAll')
    try {
      expect(
        withMaskedFieldValues(
          { root: element, mayHaveOtherFrame: false },
          () => element.value,
        ),
      ).toBe('••••••')
      expect(query).not.toHaveBeenCalled()
    } finally {
      query.mockRestore()
    }
  })
})

it('discovers conservatively when opener inspection fails unexpectedly', () => {
  const { element } = retainedField('input', { type: 'password' }, 'secret')
  page.document.body.append(element)
  withOpener(
    {
      get() {
        throw new Error('opener inspection failed')
      },
    },
    () => {
      expect(
        withMaskedFieldValues(
          { root: page.document.body, mayHaveOtherFrame: false },
          () => element.value,
        ),
      ).toBe('••••••')
    },
  )
})

it('rejects input and textarea tag names outside the HTML namespace in every guard', () => {
  const namespace = 'http://www.w3.org/2000/svg'
  const input = page.document.createElementNS(namespace, 'input')
  const textarea = page.document.createElementNS(namespace, 'textarea')
  for (const element of [input, textarea]) {
    element.setAttribute('autocomplete', 'new-password')
    element.setAttribute('value', 'public')
  }
  const text = page.document.createTextNode('public')
  textarea.append(text)
  page.document.body.append(input, textarea)
  const nativeStyle = page.document.createElement('div').style
  nativeStyle.content = '"PublicContent"'
  const style = spyOn(globalThis, 'getComputedStyle').mockReturnValue(
    nativeStyle,
  )
  try {
    expect(
      withMaskedFieldValues({ root: input, mayHaveOtherFrame: true }, () => [
        input.getAttribute('value'),
        textarea.getAttribute('value'),
        textarea.textContent,
        text.nodeValue,
        text.textContent,
        text.data,
        text.wholeText,
      ]),
    ).toEqual(Array(7).fill('public'))
    withMaskedFieldValues({ root: textarea, mayHaveOtherFrame: true }, () => {
      expect(getComputedStyle(input)).toBe(nativeStyle)
      expect(getComputedStyle(textarea, '::before')).toBe(nativeStyle)
    })
  } finally {
    style.mockRestore()
  }
})

it('deduplicates actual prototypes and restores foreign shadows across nested scopes', () => {
  const input = retainedField('input', { type: 'password' }, 'secret')
  const duplicate = field('input', { type: 'password' }, 'second')
  Object.setPrototypeOf(duplicate, input.prototype)
  const textarea = retainedField('textarea', { autocomplete: 'cc-csc' }, 'code')
  const text = textarea.element.firstChild
  if (!text) throw new Error('the textarea supplies a text child')
  const textPrototype = siblingPrototype(Text.prototype)
  Object.setPrototypeOf(text, textPrototype)
  page.document.body.append(input.element, duplicate, textarea.element)
  const targets = [input.prototype, textarea.prototype, textPrototype]
  const before = targets.map((target) =>
    Object.getOwnPropertyDescriptors(target),
  )
  const define = spyOn(Object, 'defineProperty')
  try {
    withMaskedFieldValues(
      { root: input.element, mayHaveOtherFrame: true },
      () => {
        for (const [index, count] of [3, 6, 4].entries())
          expect(
            define.mock.calls.filter(([target]) => target === targets[index]),
          ).toHaveLength(count)
        const outer = targets.map((target) =>
          Object.getOwnPropertyDescriptors(target),
        )
        expect(
          withMaskedFieldValues(
            { root: textarea.element, mayHaveOtherFrame: true },
            () => [
              input.element.value,
              duplicate.value,
              textarea.element.value,
            ],
          ),
        ).toEqual(['••••••', '••••••', '••••'])
        expect(
          targets.map((target) => Object.getOwnPropertyDescriptors(target)),
        ).toEqual(outer)
      },
    )
    expect(
      targets.map((target) => Object.getOwnPropertyDescriptors(target)),
    ).toEqual(before)
    expect([
      input.element.value,
      duplicate.value,
      textarea.element.value,
    ]).toEqual(['secret', 'second', 'code'])
  } finally {
    define.mockRestore()
  }
})

it('validates a foreign accessor before installing any local or foreign override', () => {
  const { element, prototype } = retainedField(
    'textarea',
    { type: 'password' },
    'secret',
  )
  page.document.body.append(element)
  Object.defineProperty(prototype, 'defaultValue', {
    configurable: true,
    get: undefined,
  })
  const before = descriptors()
  const foreignBefore = Object.getOwnPropertyDescriptors(prototype)
  const define = spyOn(Object, 'defineProperty')
  try {
    expect(() =>
      withMaskedFieldValues(
        { root: page.document.body, mayHaveOtherFrame: true },
        () => {
          throw new Error('walk must not run')
        },
      ),
    ).toThrow('snapshot textarea text accessor is unavailable')
    expect(define).not.toHaveBeenCalled()
    expect(descriptors()).toEqual(before)
    expect(Object.getOwnPropertyDescriptors(prototype)).toEqual(foreignBefore)
  } finally {
    define.mockRestore()
  }
})

it('refuses non-overridable foreign descriptors before installing any guard', () => {
  const { element, prototype } = retainedField(
    'input',
    { type: 'password' },
    'secret',
  )
  // getAttribute is inherited. A non-extensible retained prototype cannot own its guard.
  Object.preventExtensions(prototype)
  const before = descriptors()
  const define = spyOn(Object, 'defineProperty')
  try {
    expect(() =>
      withMaskedFieldValues({ root: element, mayHaveOtherFrame: false }, () => {
        throw new Error('walk must not run')
      }),
    ).toThrow('snapshot field getAttribute cannot be overridden')
    expect(define).not.toHaveBeenCalled()
    expect(descriptors()).toEqual(before)
    expect(element.value).toBe('secret')
  } finally {
    define.mockRestore()
  }
})

it('restores all earlier local and foreign guards if any foreign installation fails', () => {
  const input = retainedField('input', { type: 'password' }, 'secret')
  const textarea = retainedField('textarea', { autocomplete: 'cc-csc' }, 'code')
  const text = textarea.element.firstChild
  if (!text) throw new Error('the textarea supplies a text child')
  const textPrototype = siblingPrototype(Text.prototype)
  Object.setPrototypeOf(text, textPrototype)
  page.document.body.append(input.element, textarea.element)
  const properties: [object, string][] = [
    ...['value', 'defaultValue', 'getAttribute'].map(
      (name): [object, string] => [input.prototype, name],
    ),
    ...[
      'value',
      'getAttribute',
      'textContent',
      'innerText',
      'innerHTML',
      'defaultValue',
    ].map((name): [object, string] => [textarea.prototype, name]),
    ...['nodeValue', 'textContent', 'data', 'wholeText'].map(
      (name): [object, string] => [textPrototype, name],
    ),
  ]
  const targets = [input.prototype, textarea.prototype, textPrototype]
  const before = descriptors()
  const foreignBefore = targets.map((target) =>
    Object.getOwnPropertyDescriptors(target),
  )
  const define = Object.defineProperty
  for (const [target, property] of properties) {
    const install = spyOn(Object, 'defineProperty').mockImplementation(
      (object, name, descriptor) => {
        if (object === target && name === property)
          throw new Error('foreign installation refused')
        return define(object, name, descriptor)
      },
    )
    try {
      expect(() =>
        withMaskedFieldValues(
          { root: input.element, mayHaveOtherFrame: true },
          () => {
            throw new Error('walk must not run')
          },
        ),
      ).toThrow('foreign installation refused')
      expect(descriptors()).toEqual(before)
      expect(
        targets.map((prototype) => Object.getOwnPropertyDescriptors(prototype)),
      ).toEqual(foreignBefore)
    } finally {
      install.mockRestore()
    }
  }
  expect([
    input.element.value,
    textarea.element.value,
    text.textContent,
  ]).toEqual(['secret', 'code', 'code'])
})

it('attempts every local and foreign restoration in reverse after a foreign cleanup failure', () => {
  const input = retainedField('input', { type: 'password' }, 'secret')
  const textarea = retainedField('textarea', { autocomplete: 'cc-csc' }, 'code')
  const text = textarea.element.firstChild
  if (!text) throw new Error('the textarea supplies a text child')
  const textPrototype = siblingPrototype(Text.prototype)
  Object.setPrototypeOf(text, textPrototype)
  page.document.body.append(input.element, textarea.element)
  const targets = [input.prototype, textarea.prototype, textPrototype]
  const before = descriptors()
  const foreignBefore = targets.map((target) =>
    Object.getOwnPropertyDescriptors(target),
  )
  const installed: [unknown, PropertyKey][] = []
  const restored: [unknown, PropertyKey][] = []
  const define = Object.defineProperty
  const remove = Reflect.deleteProperty
  let cleaning = false
  const installation = spyOn(Object, 'defineProperty').mockImplementation(
    (object, name, descriptor) => {
      const result = define(object, name, descriptor)
      if (cleaning) {
        restored.push([object, name])
        if (object === input.prototype && name === 'value')
          throw new Error('foreign accessor restoration reported failure')
      } else installed.push([object, name])
      return result
    },
  )
  const deletion = spyOn(Reflect, 'deleteProperty').mockImplementation(
    (object, name) => {
      const result = remove(object, name)
      if (cleaning) {
        restored.push([object, name])
        if (object === textPrototype && name === 'textContent')
          throw new Error('foreign shadow removal reported failure')
      }
      return result
    },
  )
  try {
    expect(() =>
      withMaskedFieldValues(
        { root: input.element, mayHaveOtherFrame: true },
        () => {
          expect([input.element.value, textarea.element.value]).toEqual([
            '••••••',
            '••••',
          ])
          cleaning = true
          throw new Error('walk failed')
        },
      ),
    ).toThrow('snapshot field overrides could not be restored')
    expect(restored).toEqual(installed.toReversed())
    expect(descriptors()).toEqual(before)
    expect(
      targets.map((target) => Object.getOwnPropertyDescriptors(target)),
    ).toEqual(foreignBefore)
  } finally {
    installation.mockRestore()
    deletion.mockRestore()
  }
})
