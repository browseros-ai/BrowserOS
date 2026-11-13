import { afterEach, beforeEach, expect, it, spyOn } from 'bun:test'
import { asPointerTargets } from '../src/probe/pointer-targets'
import { withMaskedFieldValues } from '../src/probe/private-field-values'
import { openProbePage, type ProbePage } from './support/probe-page'

let page: ProbePage
beforeEach(() => {
  page = openProbePage('<html><body></body></html>')
})
afterEach(() => page.close())

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
      expect(withMaskedFieldValues(() => element.value)).toBe('••••')
      expect(element.value).toBe('A🔒z')
    }
  }
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
    withMaskedFieldValues(() => [
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
      expect(withMaskedFieldValues(() => secret.getAttribute(name))).toBe(
        '••••',
      )
      expect(withMaskedFieldValues(() => ordinary.getAttribute(name))).toBe(
        'A🔒z',
      )
      expect(secret.getAttribute(name)).toBe('A🔒z')
    }
    expect(
      withMaskedFieldValues(() => [
        secret.getAttribute('autocomplete'),
        secret.getAttribute('missing'),
      ]),
    ).toEqual(['new-password', null])
  }
  const element = page.document.createElement('div')
  element.setAttribute('autocomplete', 'new-password')
  element.setAttribute('value', 'ordinary-div')
  const original = descriptors().elementAttribute
  withMaskedFieldValues(() => {
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
    withMaskedFieldValues(() => ({
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

it('suppresses both CSS content reads only for pseudo-elements of private controls', () => {
  const nativeStyle = page.document.createElement('div').style
  nativeStyle.content = '"CSSSecretCanary"'
  nativeStyle.cursor = 'text'
  nativeStyle.display = 'inline-block'
  const read = spyOn(globalThis, 'getComputedStyle').mockReturnValue(
    nativeStyle,
  )
  try {
    for (const tag of ['input', 'textarea'] as const) {
      const secret = field(tag, { autocomplete: 'one-time-code' }, 'private')
      const ordinary = field(tag, {}, 'public')
      withMaskedFieldValues(() => {
        for (const pseudo of [undefined, null, ''])
          expect(getComputedStyle(secret, pseudo)).toBe(nativeStyle)
        for (const pseudo of ['::before', '::after', ':before', '::marker']) {
          const style = getComputedStyle(secret, pseudo)
          expect(style.content).toBe('none')
          expect(style.getPropertyValue('content')).toBe('none')
          expect(style.cursor).toBe('text')
          expect(style.getPropertyValue('display')).toBe('inline-block')
          expect(getComputedStyle(ordinary, pseudo)).toBe(nativeStyle)
        }
        const other = page.document.createElement('div')
        other.setAttribute('autocomplete', 'one-time-code')
        expect(getComputedStyle(other, '::before')).toBe(nativeStyle)
      })
      expect(getComputedStyle(secret, '::before')).toBe(nativeStyle)
      expect(nativeStyle.content).toBe('"CSSSecretCanary"')
    }
  } finally {
    read.mockRestore()
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
    expect(getComputedStyle(secret).cursor).toBe('pointer')
    for (const pseudo of ['::before', '::after']) {
      const style = getComputedStyle(secret, pseudo)
      expect(style.content).toBe('none')
      expect(style.getPropertyValue('content')).toBe('none')
      expect(style.cursor).toBe('text')
    }
  }
  try {
    const before = descriptors()
    for (const [outer, inner] of [
      [withMaskedFieldValues, marked],
      [marked, withMaskedFieldValues],
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
    withMaskedFieldValues(() => {
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
    withMaskedFieldValues(() => {
      expect(input.value).toBe('••••••')
      const outer = descriptors()
      expect(withMaskedFieldValues(() => input.value)).toBe('••••••')
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
        withMaskedFieldValues(() => {
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
    expect(() => withMaskedFieldValues(() => 'unreachable')).toThrow(
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
    expect(() => withMaskedFieldValues(() => 'unreachable')).toThrow(
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
    expect(() => withMaskedFieldValues(() => 'unreachable')).toThrow(
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
      withMaskedFieldValues(() => {
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
