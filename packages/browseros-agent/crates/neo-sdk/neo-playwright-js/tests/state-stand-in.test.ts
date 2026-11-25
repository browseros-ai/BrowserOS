import { describe, expect, it } from 'bun:test'
import { resolve } from 'node:path'
import { parseHTML } from 'linkedom'
import type * as VendoredAriaSnapshot from '../../neo-playwright/third_party/playwright/packages/injected/src/ariaSnapshot'
import type * as VendoredRoleSelectorEngine from '../../neo-playwright/third_party/playwright/packages/injected/src/roleSelectorEngine'
import type * as VendoredRoleUtils from '../../neo-playwright/third_party/playwright/packages/injected/src/roleUtils'
import type {
  AriaNode,
  AriaRole,
  AriaTemplateNode,
} from '../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'
import * as stubAriaSnapshot from '../src/stubs/ariaSnapshot'
import * as stubRoleSelectorEngine from '../src/stubs/roleSelectorEngine'
import * as stubRoleUtils from '../src/stubs/roleUtils'

/**
 * Bundles the unmodified vendored `roleUtils.ts` in memory with the same `@isomorphic/*`
 * alias `scripts/build.ts` uses, because `third_party/playwright/` sits outside this
 * package's tsconfig root where Bun's runtime loader resolves path aliases.
 */
async function loadVendoredRoleUtils(): Promise<typeof VendoredRoleUtils> {
  const vendorRoot = resolve(
    import.meta.dir,
    '../../neo-playwright/third_party/playwright',
  )
  const result = await Bun.build({
    entrypoints: [resolve(vendorRoot, 'packages/injected/src/roleUtils.ts')],
    target: 'browser',
    format: 'esm',
    minify: false,
    plugins: [
      {
        name: 'vendored-isomorphic-alias',
        setup(build) {
          build.onResolve({ filter: /^@isomorphic\// }, (args) => ({
            path: resolve(
              vendorRoot,
              'packages/isomorphic',
              `${args.path.slice('@isomorphic/'.length)}.ts`,
            ),
          }))
        },
      },
    ],
  })
  if (!result.success || !result.outputs[0])
    throw new AggregateError(result.logs, 'Could not bundle vendored roleUtils')
  const source = await result.outputs[0].text()
  const url = `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
  return (await import(url)) as typeof VendoredRoleUtils
}

const vendoredRoleUtils = await loadVendoredRoleUtils()

type AssertAssignable<Target, _Actual extends Target> = true

type _StubSignatureChecks = [
  AssertAssignable<
    typeof VendoredRoleSelectorEngine.createRoleEngine,
    typeof stubRoleSelectorEngine.createRoleEngine
  >,
  AssertAssignable<
    typeof VendoredAriaSnapshot.generateAriaTree,
    typeof stubAriaSnapshot.generateAriaTree
  >,
  AssertAssignable<
    typeof VendoredAriaSnapshot.matchesExpectAriaTemplate,
    typeof stubAriaSnapshot.matchesExpectAriaTemplate
  >,
  AssertAssignable<
    typeof VendoredAriaSnapshot.getAllElementsMatchingExpectAriaTemplate,
    typeof stubAriaSnapshot.getAllElementsMatchingExpectAriaTemplate
  >,
  AssertAssignable<
    typeof VendoredAriaSnapshot.renderAriaTreeAsJSON,
    typeof stubAriaSnapshot.renderAriaTreeAsJSON
  >,
  AssertAssignable<
    typeof VendoredAriaSnapshot.findNewElement,
    typeof stubAriaSnapshot.findNewElement
  >,
  AssertAssignable<
    VendoredAriaSnapshot.AriaSnapshot,
    stubAriaSnapshot.AriaSnapshot
  >,
  AssertAssignable<
    stubAriaSnapshot.AriaSnapshot,
    VendoredAriaSnapshot.AriaSnapshot
  >,
  AssertAssignable<
    VendoredAriaSnapshot.AriaTreeOptions,
    stubAriaSnapshot.AriaTreeOptions
  >,
  AssertAssignable<
    stubAriaSnapshot.AriaTreeOptions,
    VendoredAriaSnapshot.AriaTreeOptions
  >,
  AssertAssignable<
    VendoredAriaSnapshot.MatcherReceived,
    stubAriaSnapshot.MatcherReceived
  >,
  AssertAssignable<
    stubAriaSnapshot.MatcherReceived,
    VendoredAriaSnapshot.MatcherReceived
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getAriaRole,
    typeof stubRoleUtils.computeAriaRole
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getAriaRole,
    typeof stubRoleUtils.getAriaRole
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getAriaDisabled,
    typeof stubRoleUtils.getAriaDisabled
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getReadonly,
    typeof stubRoleUtils.getReadonly
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getCheckedAllowMixed,
    typeof stubRoleUtils.getCheckedAllowMixed
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getCheckedWithoutMixed,
    typeof stubRoleUtils.getCheckedWithoutMixed
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getAriaLabelledByElements,
    typeof stubRoleUtils.getAriaLabelledByElements
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.beginAriaCaches,
    typeof stubRoleUtils.beginAriaCaches
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.endAriaCaches,
    typeof stubRoleUtils.endAriaCaches
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getElementAccessibleName,
    typeof stubRoleUtils.getElementAccessibleName
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getElementAccessibleNameText,
    typeof stubRoleUtils.getElementAccessibleNameText
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getElementAccessibleDescription,
    typeof stubRoleUtils.getElementAccessibleDescription
  >,
  AssertAssignable<
    typeof VendoredRoleUtils.getElementAccessibleErrorMessage,
    typeof stubRoleUtils.getElementAccessibleErrorMessage
  >,
  AssertAssignable<
    VendoredRoleUtils.AccessibleName,
    stubRoleUtils.AccessibleName
  >,
  AssertAssignable<
    stubRoleUtils.AccessibleName,
    VendoredRoleUtils.AccessibleName
  >,
  AssertAssignable<
    VendoredRoleUtils.AccessibleDescription,
    stubRoleUtils.AccessibleDescription
  >,
  AssertAssignable<
    stubRoleUtils.AccessibleDescription,
    VendoredRoleUtils.AccessibleDescription
  >,
]

const VALID_INPUT_TYPES = new Set([
  'hidden',
  'text',
  'search',
  'tel',
  'url',
  'email',
  'password',
  'date',
  'month',
  'week',
  'time',
  'datetime-local',
  'number',
  'range',
  'color',
  'checkbox',
  'radio',
  'file',
  'submit',
  'image',
  'reset',
  'button',
])

const ALL_VALID_ARIA_ROLES: readonly AriaRole[] = [
  'alert',
  'alertdialog',
  'application',
  'article',
  'banner',
  'blockquote',
  'button',
  'caption',
  'cell',
  'checkbox',
  'code',
  'columnheader',
  'combobox',
  'complementary',
  'contentinfo',
  'definition',
  'deletion',
  'dialog',
  'directory',
  'document',
  'emphasis',
  'feed',
  'figure',
  'form',
  'generic',
  'grid',
  'gridcell',
  'group',
  'heading',
  'img',
  'insertion',
  'link',
  'list',
  'listbox',
  'listitem',
  'log',
  'main',
  'mark',
  'marquee',
  'math',
  'meter',
  'menu',
  'menubar',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'navigation',
  'none',
  'note',
  'option',
  'paragraph',
  'presentation',
  'progressbar',
  'radio',
  'radiogroup',
  'region',
  'row',
  'rowgroup',
  'rowheader',
  'scrollbar',
  'search',
  'searchbox',
  'separator',
  'slider',
  'spinbutton',
  'status',
  'strong',
  'subscript',
  'superscript',
  'switch',
  'tab',
  'table',
  'tablist',
  'tabpanel',
  'term',
  'textbox',
  'time',
  'timer',
  'toolbar',
  'tooltip',
  'tree',
  'treegrid',
  'treeitem',
]

const ARIA_READONLY_ROLES = [
  'checkbox',
  'combobox',
  'grid',
  'gridcell',
  'listbox',
  'radiogroup',
  'slider',
  'spinbutton',
  'textbox',
  'columnheader',
  'rowheader',
  'searchbox',
  'switch',
  'treegrid',
] as const

/** CSSOM `CSS.escape` implementation (https://drafts.csswg.org/cssom/#serialize-an-identifier). */
function cssEscape(value: string): string {
  const string = String(value)
  const length = string.length
  let result = ''
  const firstCodeUnit = string.charCodeAt(0)

  if (length === 1 && firstCodeUnit === 0x002d) return `\\${string}`

  for (let index = 0; index < length; index++) {
    const codeUnit = string.charCodeAt(index)
    if (codeUnit === 0x0000) {
      result += '\uFFFD'
      continue
    }
    if (
      (codeUnit >= 0x0001 && codeUnit <= 0x001f) ||
      codeUnit === 0x007f ||
      (index === 0 && codeUnit >= 0x0030 && codeUnit <= 0x0039) ||
      (index === 1 &&
        codeUnit >= 0x0030 &&
        codeUnit <= 0x0039 &&
        firstCodeUnit === 0x002d)
    ) {
      result += `\\${codeUnit.toString(16)} `
      continue
    }
    if (
      codeUnit >= 0x0080 ||
      codeUnit === 0x002d ||
      codeUnit === 0x005f ||
      (codeUnit >= 0x0030 && codeUnit <= 0x0039) ||
      (codeUnit >= 0x0041 && codeUnit <= 0x005a) ||
      (codeUnit >= 0x0061 && codeUnit <= 0x007a)
    ) {
      result += string.charAt(index)
      continue
    }
    result += `\\${string.charAt(index)}`
  }
  return result
}

type SavedDescriptor = {
  target: object
  key: PropertyKey
  descriptor: PropertyDescriptor | undefined
}

function withDom<T>(
  html: string,
  fn: (ctx: {
    document: Document
    window: ReturnType<typeof parseHTML>['window']
    byId: (id: string) => Element
  }) => T,
): T {
  const { window, document } = parseHTML(html)
  const saved: SavedDescriptor[] = []
  const patch = (
    target: object,
    key: PropertyKey,
    descriptor: PropertyDescriptor,
  ) => {
    saved.push({
      target,
      key,
      descriptor: Object.getOwnPropertyDescriptor(target, key),
    })
    Object.defineProperty(target, key, { configurable: true, ...descriptor })
  }

  const checkedState = new WeakMap<object, boolean>()
  const indeterminateState = new WeakMap<object, boolean>()

  patch(globalThis, 'CSS', {
    writable: true,
    enumerable: true,
    value: { escape: cssEscape },
  })
  patch(globalThis, 'HTMLFormElement', {
    writable: true,
    enumerable: true,
    value: window.HTMLFormElement,
  })
  patch(window.HTMLInputElement.prototype, 'type', {
    enumerable: true,
    get(this: HTMLInputElement) {
      const raw = this.getAttribute('type')?.toLowerCase() ?? ''
      return VALID_INPUT_TYPES.has(raw) ? raw : 'text'
    },
    set(this: HTMLInputElement, value: string) {
      this.setAttribute('type', value)
    },
  })
  patch(window.HTMLInputElement.prototype, 'checked', {
    enumerable: true,
    get(this: HTMLInputElement) {
      if (checkedState.has(this)) return checkedState.get(this) ?? false
      return this.hasAttribute('checked')
    },
    set(this: HTMLInputElement, value: boolean) {
      checkedState.set(this, Boolean(value))
    },
  })
  patch(window.HTMLInputElement.prototype, 'indeterminate', {
    enumerable: true,
    get(this: HTMLInputElement) {
      return indeterminateState.get(this) ?? false
    },
    set(this: HTMLInputElement, value: boolean) {
      indeterminateState.set(this, Boolean(value))
    },
  })
  patch(window.HTMLSelectElement.prototype, 'multiple', {
    enumerable: true,
    get(this: HTMLSelectElement) {
      return this.hasAttribute('multiple')
    },
  })
  patch(window.HTMLSelectElement.prototype, 'size', {
    enumerable: true,
    get(this: HTMLSelectElement) {
      const raw = this.getAttribute('size')
      if (raw === null) return 0
      const parsed = Number.parseInt(raw, 10)
      return Number.isFinite(parsed) && parsed >= 0 ? parsed : 0
    },
  })
  const tableRowsDescriptor: PropertyDescriptor = {
    enumerable: true,
    get(this: Element) {
      if (this.tagName !== 'TABLE') return undefined
      return Array.from(this.querySelectorAll('tr')).filter(
        (tr) => tr.closest('table') === this,
      )
    },
  }
  patch(window.HTMLTableElement.prototype, 'rows', tableRowsDescriptor)
  // linkedom constructs <table> as a plain HTMLElement rather than HTMLTableElement.
  patch(window.HTMLElement.prototype, 'rows', tableRowsDescriptor)

  const byId = (id: string): Element => {
    const el = document.getElementById(id)
    if (!el) throw new Error(`Element #${id} not found in fixture`)
    return el
  }

  try {
    return fn({ document, window, byId })
  } finally {
    for (let i = saved.length - 1; i >= 0; i--) {
      const entry = saved[i]
      if (!entry) continue
      if (entry.descriptor) {
        Object.defineProperty(entry.target, entry.key, entry.descriptor)
      } else {
        Reflect.deleteProperty(entry.target, entry.key)
      }
    }
  }
}

function collectElements(root: Document | ShadowRoot | Element): Element[] {
  const elements: Element[] = []
  const visit = (node: Document | ShadowRoot | Element) => {
    for (const el of Array.from(node.querySelectorAll('*'))) {
      elements.push(el)
      if (el.shadowRoot) visit(el.shadowRoot)
    }
  }
  visit(root)
  return elements
}

function assertParityOnElement(el: Element): void {
  expect(stubRoleUtils.computeAriaRole(el)).toBe(
    vendoredRoleUtils.getAriaRole(el),
  )
  expect(stubRoleUtils.getAriaDisabled(el)).toBe(
    vendoredRoleUtils.getAriaDisabled(el),
  )
  expect(stubRoleUtils.getReadonly(el)).toBe(vendoredRoleUtils.getReadonly(el))
  expect(stubRoleUtils.getCheckedAllowMixed(el)).toBe(
    vendoredRoleUtils.getCheckedAllowMixed(el),
  )
  expect(stubRoleUtils.getCheckedWithoutMixed(el)).toBe(
    vendoredRoleUtils.getCheckedWithoutMixed(el),
  )
  expect(stubRoleUtils.getAriaLabelledByElements(el)).toEqual(
    vendoredRoleUtils.getAriaLabelledByElements(el),
  )
}

function assertParityAcrossTree(root: Document | ShadowRoot | Element): number {
  const elements = collectElements(root)
  expect(elements.length).toBeGreaterThan(0)
  for (const el of elements) assertParityOnElement(el)
  return elements.length
}

describe('state stand-in: applicability role lists', () => {
  it('matches vendored getChecked* across every role in kAriaCheckedRoles and non-checked roles', () => {
    const attrs = [
      { suffix: 'true', attr: 'aria-checked="true"' },
      { suffix: 'false', attr: 'aria-checked="false"' },
      { suffix: 'mixed', attr: 'aria-checked="mixed"' },
      { suffix: 'upper', attr: 'aria-checked="TRUE"' },
      { suffix: 'absent', attr: '' },
    ]
    const roles = [
      ...vendoredRoleUtils.kAriaCheckedRoles,
      'button',
      'textbox',
      'generic',
    ]
    const html = roles
      .flatMap((role) =>
        attrs.map(
          ({ suffix, attr }) =>
            `<div id="c-${role}-${suffix}" role="${role}" ${attr}></div>`,
        ),
      )
      .join('\n')

    withDom(`<html><body>${html}</body></html>`, ({ document, byId }) => {
      assertParityAcrossTree(document)

      for (const role of vendoredRoleUtils.kAriaCheckedRoles) {
        expect(stubRoleUtils.getCheckedAllowMixed(byId(`c-${role}-true`))).toBe(
          true,
        )
        expect(
          stubRoleUtils.getCheckedWithoutMixed(byId(`c-${role}-true`)),
        ).toBe(true)
        expect(
          stubRoleUtils.getCheckedAllowMixed(byId(`c-${role}-false`)),
        ).toBe(false)
        expect(
          stubRoleUtils.getCheckedAllowMixed(byId(`c-${role}-mixed`)),
        ).toBe('mixed')
        expect(
          stubRoleUtils.getCheckedWithoutMixed(byId(`c-${role}-mixed`)),
        ).toBe(false)
        expect(
          stubRoleUtils.getCheckedAllowMixed(byId(`c-${role}-upper`)),
        ).toBe(false)
        expect(
          stubRoleUtils.getCheckedAllowMixed(byId(`c-${role}-absent`)),
        ).toBe(false)
      }
      expect(stubRoleUtils.getCheckedAllowMixed(byId('c-button-true'))).toBe(
        'error',
      )
      expect(stubRoleUtils.getCheckedWithoutMixed(byId('c-button-true'))).toBe(
        'error',
      )
    })
  })

  it('matches vendored getReadonly across every role in kAriaReadonlyRoles, native controls, and contenteditable', () => {
    const attrs = [
      { suffix: 'true', attr: 'aria-readonly="true"' },
      { suffix: 'false', attr: 'aria-readonly="false"' },
      { suffix: 'upper', attr: 'aria-readonly="TRUE"' },
      { suffix: 'absent', attr: '' },
    ]
    const roleHtml = [...ARIA_READONLY_ROLES, 'button', 'link', 'heading']
      .flatMap((role) =>
        attrs.map(
          ({ suffix, attr }) =>
            `<div id="r-${role}-${suffix}" role="${role}" ${attr}></div>`,
        ),
      )
      .join('\n')

    const html = `<html><body>
      ${roleHtml}
      <input id="in-plain">
      <input id="in-ro" readonly>
      <input id="in-aria-only" aria-readonly="true">
      <textarea id="ta-plain"></textarea>
      <textarea id="ta-ro" readonly></textarea>
      <select id="sel-plain"><option>a</option></select>
      <select id="sel-ro" readonly><option>a</option></select>
      <div id="ce-plain" contenteditable="true">editable</div>
      <div id="ce-textbox-ro" role="textbox" aria-readonly="true" contenteditable="true">ro</div>
      <div id="ce-textbox-rw" role="textbox" aria-readonly="false" contenteditable="true">rw</div>
      <div id="plain-div">plain</div>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      assertParityAcrossTree(document)

      for (const role of ARIA_READONLY_ROLES) {
        expect(stubRoleUtils.getReadonly(byId(`r-${role}-true`))).toBe(true)
        expect(stubRoleUtils.getReadonly(byId(`r-${role}-false`))).toBe(false)
        expect(stubRoleUtils.getReadonly(byId(`r-${role}-upper`))).toBe(false)
        expect(stubRoleUtils.getReadonly(byId(`r-${role}-absent`))).toBe(false)
      }
      expect(stubRoleUtils.getReadonly(byId('r-button-true'))).toBe('error')
      expect(stubRoleUtils.getReadonly(byId('in-plain'))).toBe(false)
      expect(stubRoleUtils.getReadonly(byId('in-ro'))).toBe(true)
      expect(stubRoleUtils.getReadonly(byId('in-aria-only'))).toBe(false)
      expect(stubRoleUtils.getReadonly(byId('ta-plain'))).toBe(false)
      expect(stubRoleUtils.getReadonly(byId('ta-ro'))).toBe(true)
      expect(stubRoleUtils.getReadonly(byId('sel-plain'))).toBe(false)
      expect(stubRoleUtils.getReadonly(byId('sel-ro'))).toBe(true)
      expect(stubRoleUtils.getReadonly(byId('ce-plain'))).toBe(false)
      expect(stubRoleUtils.getReadonly(byId('ce-textbox-ro'))).toBe(true)
      expect(stubRoleUtils.getReadonly(byId('ce-textbox-rw'))).toBe(false)
      expect(stubRoleUtils.getReadonly(byId('plain-div'))).toBe('error')
    })
  })

  it('matches vendored getAriaDisabled applicability across all 83 valid ARIA roles and all kAriaDisabledRoles entries', () => {
    const validRoleSet = new Set<string>(ALL_VALID_ARIA_ROLES)
    const disabledSet = new Set(vendoredRoleUtils.kAriaDisabledRoles)
    const abstractDisabledRoles = vendoredRoleUtils.kAriaDisabledRoles.filter(
      (role) => !validRoleSet.has(role),
    )
    expect(abstractDisabledRoles).toEqual(['composite', 'input', 'select'])

    const allRoles = [...ALL_VALID_ARIA_ROLES, ...abstractDisabledRoles]
    const html = [
      ...allRoles.map(
        (role) =>
          `<div id="d-${role}-true" role="${role}" aria-disabled="true"></div>` +
          `<div id="d-${role}-false" role="${role}" aria-disabled="false"></div>`,
      ),
      ...abstractDisabledRoles.map(
        (role) =>
          `<button id="d-btn-${role}-true" role="${role}" aria-disabled="true">B</button>` +
          `<button id="d-btn-${role}-false" role="${role}" aria-disabled="false">B</button>`,
      ),
    ].join('\n')

    withDom(`<html><body>${html}</body></html>`, ({ document, byId }) => {
      assertParityAcrossTree(document)

      for (const role of ALL_VALID_ARIA_ROLES) {
        const expected = disabledSet.has(role)
        expect(stubRoleUtils.getAriaDisabled(byId(`d-${role}-true`))).toBe(
          expected,
        )
        expect(stubRoleUtils.getAriaDisabled(byId(`d-${role}-false`))).toBe(
          false,
        )
      }
      for (const role of abstractDisabledRoles) {
        expect(stubRoleUtils.computeAriaRole(byId(`d-${role}-true`))).toBe(null)
        expect(stubRoleUtils.getAriaDisabled(byId(`d-${role}-true`))).toBe(
          false,
        )
        expect(stubRoleUtils.computeAriaRole(byId(`d-btn-${role}-true`))).toBe(
          'button',
        )
        expect(stubRoleUtils.getAriaDisabled(byId(`d-btn-${role}-true`))).toBe(
          true,
        )
        expect(stubRoleUtils.getAriaDisabled(byId(`d-btn-${role}-false`))).toBe(
          false,
        )
      }
    })
  })
})

describe('state stand-in: nested aria-disabled and native disabled', () => {
  it('handles ancestor inheritance, aria-disabled="false" overrides, casing, and shadow boundaries', () => {
    const html = `<html><body>
      <div id="anc-true" aria-disabled="true">
        <button id="child-btn">Btn</button>
        <div id="child-plain">Plain</div>
        <div id="child-tb" role="textbox">TB</div>
        <div id="override-false" aria-disabled="false">
          <button id="grandchild-btn">Enabled</button>
          <div id="re-true" aria-disabled="TRUE">
            <a id="re-true-link" href="/x">Link</a>
            <span id="override-mixed-case" aria-disabled="False">
              <button id="re-enabled-btn">Re-enabled</button>
            </span>
          </div>
        </div>
        <div id="bogus-val" aria-disabled="yes">
          <button id="bogus-child-btn">Still disabled via ancestor</button>
        </div>
      </div>
      <div id="shadow-host-disabled" aria-disabled="true"></div>
      <div id="shadow-host-enabled" aria-disabled="false"></div>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      const hostDisabled = byId('shadow-host-disabled')
      const shadow1 = hostDisabled.attachShadow({ mode: 'open' })
      shadow1.innerHTML = `
        <button id="s-btn">Inherited disabled</button>
        <div id="s-plain">Plain</div>
        <div aria-disabled="false">
          <button id="s-override-btn">Enabled inside shadow</button>
        </div>
        <div id="nested-host"></div>
      `
      const nestedHost = shadow1.querySelector('#nested-host')
      if (!nestedHost) throw new Error('missing #nested-host')
      const shadow2 = nestedHost.attachShadow({ mode: 'open' })
      shadow2.innerHTML = `<button id="s2-btn">Deep shadow disabled</button>`

      const hostEnabled = byId('shadow-host-enabled')
      const shadowEnabled = hostEnabled.attachShadow({ mode: 'open' })
      shadowEnabled.innerHTML = `<button id="s-en-btn">Enabled</button>`

      assertParityAcrossTree(document)

      expect(stubRoleUtils.getAriaDisabled(byId('child-btn'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('child-plain'))).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(byId('child-tb'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('grandchild-btn'))).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(byId('re-true-link'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('re-enabled-btn'))).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(byId('bogus-child-btn'))).toBe(true)

      const sBtn = shadow1.querySelector('#s-btn')
      const sPlain = shadow1.querySelector('#s-plain')
      const sOverrideBtn = shadow1.querySelector('#s-override-btn')
      const s2Btn = shadow2.querySelector('#s2-btn')
      const sEnBtn = shadowEnabled.querySelector('#s-en-btn')
      if (!sBtn || !sPlain || !sOverrideBtn || !s2Btn || !sEnBtn)
        throw new Error('missing shadow elements')

      expect(stubRoleUtils.getAriaDisabled(sBtn)).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(sPlain)).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(sOverrideBtn)).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(s2Btn)).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(sEnBtn)).toBe(false)
    })
  })

  it('handles native disabled controls, optgroup disabled, and fieldset disabled with first-legend exemption', () => {
    const html = `<html><body>
      <button id="n-btn" disabled>B</button>
      <input id="n-in" disabled>
      <select id="n-sel" disabled><option id="n-sel-opt">O</option></select>
      <textarea id="n-ta" disabled></textarea>
      <select id="og-sel">
        <optgroup id="og-dis" disabled>
          <option id="og-dis-opt">Disabled via optgroup</option>
        </optgroup>
        <optgroup id="og-en">
          <option id="og-en-opt-dis" disabled>Self disabled</option>
          <option id="og-en-opt">Enabled</option>
        </optgroup>
      </select>
      <fieldset id="fs-dis" disabled>
        <legend>
          <input id="fs-first-legend-in">
          <button id="fs-first-legend-btn">Exempt</button>
        </legend>
        <input id="fs-body-in">
        <legend>
          <input id="fs-second-legend-in">
        </legend>
        <fieldset id="fs-nested-en">
          <legend><input id="fs-nested-en-legend-in"></legend>
          <input id="fs-nested-en-body-in">
        </fieldset>
        <fieldset id="fs-nested-dis" disabled>
          <legend><input id="fs-nested-dis-legend-in"></legend>
          <input id="fs-nested-dis-body-in">
        </fieldset>
      </fieldset>
      <fieldset id="fs-no-legend" disabled>
        <button id="fs-no-legend-btn">Disabled</button>
      </fieldset>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      assertParityAcrossTree(document)

      expect(stubRoleUtils.getAriaDisabled(byId('n-btn'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('n-in'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('n-sel'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('n-ta'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('og-dis'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('og-dis-opt'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('og-en-opt-dis'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('og-en-opt'))).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(byId('fs-first-legend-in'))).toBe(
        false,
      )
      expect(stubRoleUtils.getAriaDisabled(byId('fs-first-legend-btn'))).toBe(
        false,
      )
      expect(stubRoleUtils.getAriaDisabled(byId('fs-body-in'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('fs-second-legend-in'))).toBe(
        true,
      )
      expect(
        stubRoleUtils.getAriaDisabled(byId('fs-nested-en-legend-in')),
      ).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('fs-nested-en-body-in'))).toBe(
        true,
      )
      expect(stubRoleUtils.getAriaDisabled(byId('fs-nested-dis-body-in'))).toBe(
        true,
      )
      expect(stubRoleUtils.getAriaDisabled(byId('fs-no-legend-btn'))).toBe(true)
    })
  })
})

describe('state stand-in: checked, mixed, and indeterminate inputs', () => {
  it('handles checkbox, radio, text indeterminate, role overrides, and option elements', () => {
    const html = `<html><body>
      <input id="cb-off" type="checkbox">
      <input id="cb-on" type="checkbox" checked>
      <input id="cb-ind-off" type="checkbox">
      <input id="cb-ind-on" type="checkbox" checked>
      <input id="cb-upper" type="CHECKBOX" checked>
      <input id="cb-role-btn" type="checkbox" role="button" checked>
      <input id="rad-off" type="radio">
      <input id="rad-on" type="radio" checked>
      <input id="rad-ind" type="radio" checked>
      <input id="txt-ind" type="text">
      <select>
        <option id="opt-true" aria-checked="true">A</option>
        <option id="opt-mixed" aria-checked="mixed">B</option>
        <option id="opt-absent">C</option>
      </select>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      ;(byId('cb-ind-off') as HTMLInputElement).indeterminate = true
      ;(byId('cb-ind-on') as HTMLInputElement).indeterminate = true
      ;(byId('rad-ind') as HTMLInputElement).indeterminate = true
      ;(byId('txt-ind') as HTMLInputElement).indeterminate = true

      assertParityAcrossTree(document)

      expect(stubRoleUtils.getCheckedAllowMixed(byId('cb-off'))).toBe(false)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('cb-on'))).toBe(true)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('cb-ind-off'))).toBe(
        'mixed',
      )
      expect(stubRoleUtils.getCheckedWithoutMixed(byId('cb-ind-off'))).toBe(
        false,
      )
      expect(stubRoleUtils.getCheckedAllowMixed(byId('cb-ind-on'))).toBe(
        'mixed',
      )
      expect(stubRoleUtils.getCheckedWithoutMixed(byId('cb-ind-on'))).toBe(true)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('cb-upper'))).toBe(true)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('cb-role-btn'))).toBe(true)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('rad-off'))).toBe(false)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('rad-on'))).toBe(true)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('rad-ind'))).toBe('mixed')
      expect(stubRoleUtils.getCheckedWithoutMixed(byId('rad-ind'))).toBe(true)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('txt-ind'))).toBe('mixed')
      expect(stubRoleUtils.getCheckedWithoutMixed(byId('txt-ind'))).toBe(
        'error',
      )
      expect(stubRoleUtils.getCheckedAllowMixed(byId('opt-true'))).toBe(true)
      expect(stubRoleUtils.getCheckedAllowMixed(byId('opt-mixed'))).toBe(
        'mixed',
      )
      expect(stubRoleUtils.getCheckedWithoutMixed(byId('opt-mixed'))).toBe(
        false,
      )
      expect(stubRoleUtils.getCheckedAllowMixed(byId('opt-absent'))).toBe(false)
    })
  })
})

describe('state stand-in: aria-labelledby resolution', () => {
  it('handles missing, whitespace, multi-ID ordering, deduplication, duplicate IDs, CSS escaping, chains, and shadow boundaries', () => {
    const html = `<html><body>
      <span id="a">Alpha</span>
      <span id="b">Beta</span>
      <span id="dup">First</span>
      <span id="dup">Second</span>
      <span id="lbl:one.two">Escaped punctuation</span>
      <span id="1-num">Escaped leading digit</span>
      <div id="missing-attr"></div>
      <div id="empty-attr" aria-labelledby=""></div>
      <div id="ws-attr" aria-labelledby="   "></div>
      <div id="unknown-ids" aria-labelledby="nope still-nope"></div>
      <div id="single" aria-labelledby="a"></div>
      <div id="ordered" aria-labelledby="b a"></div>
      <div id="dedup" aria-labelledby="a b a b"></div>
      <div id="mixed" aria-labelledby="missing a also-missing b"></div>
      <div id="dup-ref" aria-labelledby="dup"></div>
      <div id="escaped" aria-labelledby="lbl:one.two 1-num"></div>
      <div id="chain-a">Origin</div>
      <div id="chain-b" aria-labelledby="chain-a">Mid</div>
      <div id="chain-c" aria-labelledby="chain-b">End</div>
      <div id="self-ref" aria-labelledby="self-ref">Self</div>
      <div id="light-to-shadow" aria-labelledby="in-shadow"></div>
      <div id="shadow-host"></div>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      const host = byId('shadow-host')
      const shadow = host.attachShadow({ mode: 'open' })
      shadow.innerHTML = `
        <span id="in-shadow">Shadow Label</span>
        <button id="shadow-valid" aria-labelledby="in-shadow">S</button>
        <button id="shadow-cross" aria-labelledby="a">Cross</button>
      `

      assertParityAcrossTree(document)

      expect(
        stubRoleUtils.getAriaLabelledByElements(byId('missing-attr')),
      ).toBe(null)
      expect(stubRoleUtils.getAriaLabelledByElements(byId('empty-attr'))).toBe(
        null,
      )
      expect(stubRoleUtils.getAriaLabelledByElements(byId('ws-attr'))).toBe(
        null,
      )
      expect(stubRoleUtils.getAriaLabelledByElements(byId('unknown-ids'))).toBe(
        null,
      )
      expect(stubRoleUtils.getAriaLabelledByElements(byId('single'))).toEqual([
        byId('a'),
      ])
      expect(stubRoleUtils.getAriaLabelledByElements(byId('ordered'))).toEqual([
        byId('b'),
        byId('a'),
      ])
      expect(stubRoleUtils.getAriaLabelledByElements(byId('dedup'))).toEqual([
        byId('a'),
        byId('b'),
      ])
      expect(stubRoleUtils.getAriaLabelledByElements(byId('mixed'))).toEqual([
        byId('a'),
        byId('b'),
      ])

      const allDups = Array.from(document.querySelectorAll('#dup'))
      expect(allDups.length).toBe(2)
      expect(stubRoleUtils.getAriaLabelledByElements(byId('dup-ref'))).toEqual([
        allDups[0],
      ])
      expect(stubRoleUtils.getAriaLabelledByElements(byId('escaped'))).toEqual([
        byId('lbl:one.two'),
        byId('1-num'),
      ])
      expect(stubRoleUtils.getAriaLabelledByElements(byId('chain-c'))).toEqual([
        byId('chain-b'),
      ])
      expect(stubRoleUtils.getAriaLabelledByElements(byId('chain-b'))).toEqual([
        byId('chain-a'),
      ])
      expect(stubRoleUtils.getAriaLabelledByElements(byId('self-ref'))).toEqual(
        [byId('self-ref')],
      )
      expect(
        stubRoleUtils.getAriaLabelledByElements(byId('light-to-shadow')),
      ).toBe(null)

      const sLabel = shadow.querySelector('#in-shadow')
      const sValid = shadow.querySelector('#shadow-valid')
      const sCross = shadow.querySelector('#shadow-cross')
      if (!sLabel || !sValid || !sCross)
        throw new Error('missing shadow elements')

      expect(stubRoleUtils.getAriaLabelledByElements(sValid)).toEqual([sLabel])
      expect(stubRoleUtils.getAriaLabelledByElements(sCross)).toBe(null)

      const detached = document.createElement('div')
      detached.setAttribute('aria-labelledby', 'a')
      assertParityOnElement(detached)
      expect(stubRoleUtils.getAriaLabelledByElements(detached)).toBe(null)
    })
  })
})

describe('state stand-in: computeAriaRole edge cases', () => {
  it('matches vendored getAriaRole across cover roles, static tags, and dynamic HTML elements', () => {
    const html = `<html><body>
      <header id="hdr-top">Top</header>
      <footer id="ftr-top">Bottom</footer>
      <nav id="nav-el"><header id="hdr-in-nav"></header><footer id="ftr-in-nav"></footer></nav>
      <main id="main-el"><header id="hdr-in-main"></header></main>
      <aside id="aside-el"><footer id="ftr-in-aside"></footer></aside>
      <article id="art-el"><header id="hdr-in-art"></header></article>
      <section id="sec-unnamed"><header id="hdr-in-sec"></header></section>
      <section id="sec-named" aria-label="Info"></section>
      <section id="sec-labelled" aria-labelledby="hdr-top"></section>
      <div role="region"><footer id="ftr-in-role-region"></footer></div>
      <article role="presentation"><header id="hdr-in-pres-art"></header></article>
      <form id="form-unnamed"></form>
      <form id="form-named" aria-label="Search"></form>
      <form id="form-empty-label" aria-label=""></form>
      <a id="a-href" href="https://example.com">Link</a>
      <a id="a-nohref">Anchor</a>
      <map><area id="area-href" href="/x"><area id="area-nohref"></map>
      <img id="img-noalt">
      <img id="img-alt" alt="pic">
      <img id="img-empty-alt" alt="">
      <img id="img-empty-alt-title" alt="" title="t">
      <img id="img-empty-alt-empty-title" alt="" title="">
      <img id="img-empty-alt-tabindex" alt="" tabindex="0">
      <img id="img-empty-alt-busy" alt="" aria-busy="true">
      <img id="img-empty-alt-label" alt="" aria-label="icon">
      <datalist id="dl-valid"><option>One</option></datalist>
      <div id="not-datalist"></div>
      <input id="in-default">
      <input id="in-text" type="text">
      <input id="in-search" type="search">
      <input id="in-email" type="email">
      <input id="in-tel" type="tel">
      <input id="in-url" type="url">
      <input id="in-list-dl" type="text" list="dl-valid">
      <input id="in-search-dl" type="search" list="dl-valid">
      <input id="in-list-div" type="text" list="not-datalist">
      <input id="in-checkbox" type="checkbox">
      <input id="in-radio" type="radio">
      <input id="in-button" type="button">
      <input id="in-submit" type="submit">
      <input id="in-reset" type="reset">
      <input id="in-image" type="image">
      <input id="in-number" type="number">
      <input id="in-range" type="range">
      <input id="in-file" type="file">
      <input id="in-hidden" type="hidden">
      <input id="in-password" type="password">
      <input id="in-invalid" type="foobar">
      <select id="sel-combo"><option>1</option></select>
      <select id="sel-multi" multiple><option>1</option></select>
      <select id="sel-size1" size="1"><option>1</option></select>
      <select id="sel-size3" size="3"><option>1</option></select>
      <dialog id="dlg">D</dialog>
      <div id="r-alertdialog" role="alertdialog"></div>
      <div id="r-menu" role="menu"></div>
      <div id="r-listbox" role="listbox"></div>
      <div id="r-tooltip" role="tooltip"></div>
      <div id="r-status" role="status"></div>
      <div id="r-alert" role="alert"></div>
      <div id="r-log" role="log"></div>
      <output id="out-el"></output>
      <search id="search-el"></search>
      <blockquote><p>P</p></blockquote>
      <button>B</button>
      <code>C</code><del>D</del><ins>I</ins><em>E</em><strong>S</strong>
      <sub>1</sub><sup>2</sup><mark>M</mark><time>T</time><hr>
      <details><summary>S</summary></details>
      <fieldset><legend>L</legend></fieldset>
      <figure><figcaption>F</figcaption></figure>
      <h1>1</h1><h2>2</h2><h3>3</h3><h4>4</h4><h5>5</h5><h6>6</h6>
      <dl><dt><dfn>Term</dfn></dt><dd>Def</dd></dl>
      <ul><li>Item</li></ul><ol><li>Item</li></ol><menu><li>Item</li></menu>
      <meter value="0.5"></meter><progress value="0.5"></progress>
      <math></math><svg></svg><textarea></textarea>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      assertParityAcrossTree(document)

      expect(stubRoleUtils.computeAriaRole(byId('hdr-top'))).toBe('banner')
      expect(stubRoleUtils.computeAriaRole(byId('ftr-top'))).toBe('contentinfo')
      expect(stubRoleUtils.computeAriaRole(byId('hdr-in-nav'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('ftr-in-nav'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('hdr-in-main'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('ftr-in-aside'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('hdr-in-art'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('hdr-in-sec'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('ftr-in-role-region'))).toBe(
        null,
      )
      expect(stubRoleUtils.computeAriaRole(byId('hdr-in-pres-art'))).toBe(
        'banner',
      )
      expect(stubRoleUtils.computeAriaRole(byId('sec-unnamed'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('sec-named'))).toBe('region')
      expect(stubRoleUtils.computeAriaRole(byId('sec-labelled'))).toBe('region')
      expect(stubRoleUtils.computeAriaRole(byId('form-unnamed'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('form-named'))).toBe('form')
      expect(stubRoleUtils.computeAriaRole(byId('form-empty-label'))).toBe(
        'form',
      )
      expect(stubRoleUtils.computeAriaRole(byId('a-href'))).toBe('link')
      expect(stubRoleUtils.computeAriaRole(byId('a-nohref'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('area-href'))).toBe('link')
      expect(stubRoleUtils.computeAriaRole(byId('area-nohref'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('img-empty-alt'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('img-alt'))).toBe('img')
      expect(stubRoleUtils.computeAriaRole(byId('img-empty-alt-title'))).toBe(
        'img',
      )
      expect(
        stubRoleUtils.computeAriaRole(byId('img-empty-alt-tabindex')),
      ).toBe('img')
      expect(stubRoleUtils.computeAriaRole(byId('img-empty-alt-busy'))).toBe(
        'img',
      )
      expect(stubRoleUtils.computeAriaRole(byId('in-list-dl'))).toBe('combobox')
      expect(stubRoleUtils.computeAriaRole(byId('in-search-dl'))).toBe(
        'combobox',
      )
      expect(stubRoleUtils.computeAriaRole(byId('in-list-div'))).toBe('textbox')
      expect(stubRoleUtils.computeAriaRole(byId('in-search'))).toBe('searchbox')
      expect(stubRoleUtils.computeAriaRole(byId('in-number'))).toBe(
        'spinbutton',
      )
      expect(stubRoleUtils.computeAriaRole(byId('in-range'))).toBe('slider')
      expect(stubRoleUtils.computeAriaRole(byId('in-file'))).toBe('button')
      expect(stubRoleUtils.computeAriaRole(byId('in-hidden'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('in-invalid'))).toBe('textbox')
      expect(stubRoleUtils.computeAriaRole(byId('sel-combo'))).toBe('combobox')
      expect(stubRoleUtils.computeAriaRole(byId('sel-multi'))).toBe('listbox')
      expect(stubRoleUtils.computeAriaRole(byId('sel-size1'))).toBe('combobox')
      expect(stubRoleUtils.computeAriaRole(byId('sel-size3'))).toBe('listbox')
      expect(stubRoleUtils.computeAriaRole(byId('dlg'))).toBe('dialog')
      expect(stubRoleUtils.computeAriaRole(byId('out-el'))).toBe('status')
      expect(stubRoleUtils.computeAriaRole(byId('search-el'))).toBe('search')
    })
  })

  it('matches vendored table cell (td/th) heuristics across plain, grid, and treegrid tables', () => {
    const html = `<html><body>
      <table><caption>Cap</caption><tr><td id="td-plain">A</td></tr></table>
      <table role="grid"><tr><td id="td-grid">G</td></tr></table>
      <table role="treegrid"><tr><td id="td-treegrid">TG</td></tr></table>
      <table role="table"><tr><td id="td-table">T</td></tr></table>

      <table>
        <tr>
          <th id="th-scope-col" scope="col">C</th>
          <th id="th-scope-colgroup" scope="colgroup">CG</th>
          <th id="th-scope-row" scope="row">R</th>
          <th id="th-scope-rowgroup" scope="rowgroup">RG</th>
        </tr>
      </table>

      <table>
        <tr><th id="th-lone-1row">Lone</th></tr>
      </table>

      <table>
        <tr><th id="th-lone-2row">Col</th></tr>
        <tr><td>Val</td></tr>
      </table>

      <table>
        <tr><th id="th-prev">A</th><th id="th-mid">B</th><th id="th-next">C</th></tr>
        <tr><td>1</td><td>2</td><td>3</td></tr>
      </table>

      <table>
        <tr><th id="th-nonempty-td">Row</th><td>Data</td></tr>
        <tr><th id="th-elem-td">Row2</th><td><span></span></td></tr>
        <tr><th id="th-empty-td">ColDefault</th><td>   </td></tr>
      </table>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      assertParityAcrossTree(document)

      expect(stubRoleUtils.computeAriaRole(byId('td-plain'))).toBe('cell')
      expect(stubRoleUtils.computeAriaRole(byId('td-grid'))).toBe('gridcell')
      expect(stubRoleUtils.computeAriaRole(byId('td-treegrid'))).toBe(
        'gridcell',
      )
      expect(stubRoleUtils.computeAriaRole(byId('td-table'))).toBe('cell')
      expect(stubRoleUtils.computeAriaRole(byId('th-scope-col'))).toBe(
        'columnheader',
      )
      expect(stubRoleUtils.computeAriaRole(byId('th-scope-colgroup'))).toBe(
        'columnheader',
      )
      expect(stubRoleUtils.computeAriaRole(byId('th-scope-row'))).toBe(
        'rowheader',
      )
      expect(stubRoleUtils.computeAriaRole(byId('th-scope-rowgroup'))).toBe(
        'rowheader',
      )
      expect(stubRoleUtils.computeAriaRole(byId('th-lone-1row'))).toBe(null)
      expect(stubRoleUtils.computeAriaRole(byId('th-lone-2row'))).toBe(
        'columnheader',
      )
      expect(stubRoleUtils.computeAriaRole(byId('th-mid'))).toBe('columnheader')
      expect(stubRoleUtils.computeAriaRole(byId('th-nonempty-td'))).toBe(
        'rowheader',
      )
      expect(stubRoleUtils.computeAriaRole(byId('th-elem-td'))).toBe(
        'rowheader',
      )
      expect(stubRoleUtils.computeAriaRole(byId('th-empty-td'))).toBe(
        'columnheader',
      )

      const detachedTh = document.createElement('th')
      assertParityOnElement(detachedTh)
      expect(stubRoleUtils.computeAriaRole(detachedTh)).toBe('columnheader')
    })
  })

  it('matches vendored multi-token role fallbacks, presentation conflict resolution, and presentation inheritance', () => {
    const html = `<html><body>
      <div id="multi-valid" role="heading button" aria-disabled="true"></div>
      <div id="multi-fallback" role="invalid button" aria-disabled="true"></div>
      <button id="multi-all-invalid" role="notarole bogus"></button>
      <button id="override-non-disabled-role" role="heading" aria-disabled="true">H</button>
      <fieldset disabled><div id="fs-dis-aria-btn" role="button">Not native form control</div></fieldset>

      <h1 id="pres-plain" role="presentation">H</h1>
      <h1 id="pres-tab0" role="presentation" tabindex="0">H</h1>
      <h1 id="pres-tab-empty" role="presentation" tabindex="">H</h1>
      <h1 id="pres-tab-abc" role="presentation" tabindex="abc">H</h1>
      <a id="pres-a-href" href="/x" role="presentation">L</a>
      <a id="pres-a-nohref" role="presentation">L</a>
      <button id="pres-btn-enabled" role="none">B</button>
      <button id="pres-btn-disabled" disabled role="none">B</button>
      <input id="pres-in-hidden" hidden role="presentation">
      <p id="pres-p-label" role="presentation" aria-label="prohibited">P</p>
      <p id="pres-p-busy" role="presentation" aria-busy="true">P</p>
      <h1 id="pres-h1-label" role="presentation" aria-label="allowed">H</h1>
      <div id="pres-div-roledesc" role="none" aria-roledescription="card">D</div>

      <ul id="ul-pres" role="presentation"><li id="li-pres">Item</li></ul>
      <ol id="ol-none" role="none"><li id="li-none">Item</li></ol>
      <ul id="ul-pres-focus" role="presentation" tabindex="0"><li id="li-no-inherit">Item</li></ul>
      <ul id="ul-pres-label" role="presentation" aria-label="x"><li id="li-pres-label">Item</li></ul>
      <ul id="ul-none-label" role="none" aria-label="x"><li id="li-none-label">Item</li></ul>
      <dl role="presentation"><div><dt id="dt-pres">T</dt><dd id="dd-pres">D</dd></div></dl>

      <table id="tbl-pres" role="presentation">
        <thead><tr id="tr-head-pres" aria-disabled="true"><th id="th-pres" aria-disabled="true">H</th></tr></thead>
        <tbody><tr id="tr-body-pres" aria-disabled="true"><td id="td-pres" aria-disabled="true">D</td></tr></tbody>
      </table>
      <table role="grid">
        <tbody><tr id="tr-grid-dis" aria-disabled="true"><td id="td-grid-dis" aria-disabled="true">D</td></tr></tbody>
      </table>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      assertParityAcrossTree(document)

      expect(stubRoleUtils.computeAriaRole(byId('multi-valid'))).toBe('heading')
      expect(stubRoleUtils.getAriaDisabled(byId('multi-valid'))).toBe(false)
      expect(stubRoleUtils.computeAriaRole(byId('multi-fallback'))).toBe(
        'button',
      )
      expect(stubRoleUtils.getAriaDisabled(byId('multi-fallback'))).toBe(true)
      expect(stubRoleUtils.computeAriaRole(byId('multi-all-invalid'))).toBe(
        'button',
      )
      expect(
        stubRoleUtils.computeAriaRole(byId('override-non-disabled-role')),
      ).toBe('heading')
      expect(
        stubRoleUtils.getAriaDisabled(byId('override-non-disabled-role')),
      ).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(byId('fs-dis-aria-btn'))).toBe(false)
      expect(stubRoleUtils.computeAriaRole(byId('pres-plain'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-tab0'))).toBe('heading')
      expect(stubRoleUtils.computeAriaRole(byId('pres-tab-empty'))).toBe(
        'heading',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-tab-abc'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-a-href'))).toBe('link')
      expect(stubRoleUtils.computeAriaRole(byId('pres-a-nohref'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-btn-enabled'))).toBe(
        'button',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-btn-disabled'))).toBe(
        'none',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-in-hidden'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-p-label'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-p-busy'))).toBe(
        'paragraph',
      )
      expect(stubRoleUtils.computeAriaRole(byId('pres-h1-label'))).toBe(
        'heading',
      )
      expect(stubRoleUtils.computeAriaRole(byId('li-pres'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('li-none'))).toBe('none')
      expect(stubRoleUtils.computeAriaRole(byId('li-no-inherit'))).toBe(
        'listitem',
      )
      expect(stubRoleUtils.computeAriaRole(byId('li-pres-label'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('li-none-label'))).toBe(
        'listitem',
      )
      expect(stubRoleUtils.computeAriaRole(byId('dt-pres'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('dd-pres'))).toBe(
        'presentation',
      )

      expect(stubRoleUtils.computeAriaRole(byId('tr-head-pres'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('th-pres'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.computeAriaRole(byId('td-pres'))).toBe(
        'presentation',
      )
      expect(stubRoleUtils.getAriaDisabled(byId('tr-head-pres'))).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(byId('th-pres'))).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(byId('td-pres'))).toBe(false)
      expect(stubRoleUtils.getAriaDisabled(byId('tr-grid-dis'))).toBe(true)
      expect(stubRoleUtils.getAriaDisabled(byId('td-grid-dis'))).toBe(true)
    })
  })
})

describe('state stand-in: stub contracts', () => {
  it('returns null from getAriaRole, keeps cache hooks no-op, and throws from name/description/error functions', () => {
    const html = `<html><body>
      <button id="btn">Click</button>
      <div id="role-btn" role="button" aria-label="Act"></div>
      <a id="lnk" href="/home">Home</a>
      <input id="chk" type="checkbox" checked>
    </body></html>`

    withDom(html, ({ document, byId }) => {
      for (const el of collectElements(document))
        expect(stubRoleUtils.getAriaRole(el)).toBe(null)

      expect(stubRoleUtils.endAriaCaches()).toBeUndefined()
      expect(stubRoleUtils.beginAriaCaches()).toBeUndefined()
      expect(stubRoleUtils.beginAriaCaches()).toBeUndefined()
      expect(stubRoleUtils.endAriaCaches()).toBeUndefined()
      expect(stubRoleUtils.endAriaCaches()).toBeUndefined()
      expect(stubRoleUtils.endAriaCaches()).toBeUndefined()

      const btn = byId('btn')
      expect(() => stubRoleUtils.getElementAccessibleName(btn, false)).toThrow(
        /Rust/,
      )
      expect(() =>
        stubRoleUtils.getElementAccessibleNameText(btn, true),
      ).toThrow(/Rust/)
      expect(() =>
        stubRoleUtils.getElementAccessibleDescription(btn, false),
      ).toThrow(/Rust/)
      expect(() => stubRoleUtils.getElementAccessibleErrorMessage(btn)).toThrow(
        /Rust/,
      )
    })
  })

  it('throws from all five ariaSnapshot stand-in functions', () => {
    withDom('<html><body><main id="m"></main></body></html>', ({ byId }) => {
      const root = byId('m')
      const template: AriaTemplateNode = { kind: 'role', role: 'fragment' }
      const node: AriaNode = {
        role: 'fragment',
        name: '',
        children: [],
        box: { visible: true, inline: false },
        receivesPointerEvents: true,
        props: {},
      }
      const snapshot: stubAriaSnapshot.AriaSnapshot = {
        root: node,
        info: new Map(),
        refs: new Map(),
        iframeRefs: [],
      }

      expect(() =>
        stubAriaSnapshot.generateAriaTree(root, { mode: 'default' }),
      ).toThrow(/Rust/)
      expect(() =>
        stubAriaSnapshot.matchesExpectAriaTemplate(root, template),
      ).toThrow(/Rust/)
      expect(() =>
        stubAriaSnapshot.getAllElementsMatchingExpectAriaTemplate(
          root,
          template,
        ),
      ).toThrow(/Rust/)
      expect(() =>
        stubAriaSnapshot.renderAriaTreeAsJSON(snapshot, { mode: 'ai' }),
      ).toThrow(/Rust/)
      expect(() => stubAriaSnapshot.findNewElement(undefined, node)).toThrow(
        /Rust/,
      )
    })
  })

  it('constructs role and internal:role engines without throwing and throws from queryAll', () => {
    withDom('<html><body><button>Ok</button></body></html>', ({ document }) => {
      const publicEngine = stubRoleSelectorEngine.createRoleEngine(false)
      const internalEngine = stubRoleSelectorEngine.createRoleEngine(true)

      expect(() => publicEngine.queryAll(document, 'button')).toThrow(
        /'role'.*Rust/,
      )
      expect(() => internalEngine.queryAll(document, 'button')).toThrow(
        /'internal:role'.*Rust/,
      )
    })
  })

  it('boots unmodified vendored InjectedScript bundled with the three stand-ins', async () => {
    const vendorRoot = resolve(
      import.meta.dir,
      '../../neo-playwright/third_party/playwright',
    )
    const stubsDir = resolve(import.meta.dir, '../src/stubs')
    const stubMap: Record<string, string> = {
      './highlight': resolve(stubsDir, 'highlight.ts'),
      './consoleApi': resolve(stubsDir, 'consoleApi.ts'),
      './roleUtils': resolve(stubsDir, 'roleUtils.ts'),
      './ariaSnapshot': resolve(stubsDir, 'ariaSnapshot.ts'),
      './roleSelectorEngine': resolve(stubsDir, 'roleSelectorEngine.ts'),
    }
    const build = await Bun.build({
      entrypoints: [
        resolve(vendorRoot, 'packages/injected/src/injectedScript.ts'),
      ],
      target: 'browser',
      format: 'esm',
      minify: false,
      plugins: [
        {
          name: 'stand-in-swap-proof',
          setup(pluginBuild) {
            pluginBuild.onResolve(
              {
                filter:
                  /^\.\/(highlight|consoleApi|roleUtils|ariaSnapshot|roleSelectorEngine)$/,
              },
              (args) => {
                const target = stubMap[args.path]
                return target ? { path: target } : undefined
              },
            )
            pluginBuild.onResolve({ filter: /^@isomorphic\// }, (args) => ({
              path: resolve(
                vendorRoot,
                'packages/isomorphic',
                `${args.path.slice('@isomorphic/'.length)}.ts`,
              ),
            }))
          },
        },
      ],
    })
    if (!build.success || !build.outputs[0])
      throw new AggregateError(
        build.logs,
        'Could not bundle vendored InjectedScript with stand-ins',
      )
    const source = await build.outputs[0].text()
    expect(source).not.toContain('ariaSnapshotDistiller')
    const url = `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`
    const mod = (await import(url)) as {
      InjectedScript: new (
        win: unknown,
        options: Record<string, unknown>,
      ) => {
        elementState(
          el: Element,
          state: string,
        ): { matches: boolean; received?: string }
        parseSelector(selector: string): unknown
        querySelectorAll(parsed: unknown, root: Node): Element[]
        generateSelectorSimple(el: Element): string
        ariaSnapshotJSON(node: Node, options: { mode: string }): unknown
      }
    }

    const html = `<html><body>
      <span id="lbl-user">Username</span>
      <input id="user" type="text" aria-labelledby="lbl-user" data-testid="user-input">
      <input id="ro" type="text" readonly>
      <input id="chk" type="checkbox" checked>
      <div id="dis-wrap" aria-disabled="true"><button id="dis-btn">Save</button></div>
    </body></html>`

    withDom(html, ({ window, document, byId }) => {
      const savedGlobals: [string, PropertyDescriptor | undefined][] = []
      const domGlobals = [
        'window',
        'document',
        'Node',
        'Element',
        'HTMLElement',
        'HTMLInputElement',
        'HTMLTextAreaElement',
        'HTMLIFrameElement',
        'Document',
        'DocumentFragment',
        'ShadowRoot',
        'Text',
        'Attr',
        'CharacterData',
        'MutationObserver',
        'CustomEvent',
        'Event',
        'getComputedStyle',
      ]
      for (const name of domGlobals) {
        savedGlobals.push([
          name,
          Object.getOwnPropertyDescriptor(globalThis, name),
        ])
        Object.defineProperty(globalThis, name, {
          configurable: true,
          writable: true,
          value: Reflect.get(window, name),
        })
      }
      try {
        const injected = new mod.InjectedScript(window, {
          isUnderTest: false,
          sdkLanguage: 'javascript',
          frameSeq: 0,
          testIdAttributeName: 'data-testid',
          stableRafCount: 1,
          browserName: 'chromium',
          isUtilityWorld: true,
          customEngines: [],
        })

        expect(injected.elementState(byId('dis-btn'), 'disabled').matches).toBe(
          true,
        )
        expect(injected.elementState(byId('user'), 'editable').matches).toBe(
          true,
        )
        expect(injected.elementState(byId('ro'), 'editable').matches).toBe(
          false,
        )
        expect(injected.elementState(byId('chk'), 'checked').matches).toBe(true)

        const byLabel = injected.querySelectorAll(
          injected.parseSelector('internal:label="Username"'),
          document,
        )
        expect(byLabel).toEqual([byId('user')])

        expect(injected.generateSelectorSimple(byId('user'))).toBe(
          'internal:testid=[data-testid="user-input"s]',
        )

        expect(() =>
          injected.querySelectorAll(
            injected.parseSelector('internal:role=button'),
            document,
          ),
        ).toThrow(/'internal:role'.*Rust/)
        expect(() =>
          injected.querySelectorAll(
            injected.parseSelector('role=button'),
            document,
          ),
        ).toThrow(/'role'.*Rust/)
        expect(() =>
          injected.ariaSnapshotJSON(document.body, { mode: 'ai' }),
        ).toThrow(/Rust/)
      } finally {
        for (let i = savedGlobals.length - 1; i >= 0; i--) {
          const entry = savedGlobals[i]
          if (!entry) continue
          const [name, desc] = entry
          if (desc) Object.defineProperty(globalThis, name, desc)
          else Reflect.deleteProperty(globalThis, name)
        }
      }
    })
  })
})
