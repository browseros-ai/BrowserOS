/**
 * Copyright (c) Microsoft Corporation.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
// Build-time stand-in for upstream roleUtils.ts. Keeps live DOM state checks and pure-DOM
// role computation (`computeAriaRole`, for state checks and cover decisions) in the page
// without layout, CSS tokenization, caches, or accessible-name computation.
import {
  closestCrossShadow,
  elementSafeTagName,
  enclosingShadowRootOrDocument,
  isListBoxSelect,
  parentElementOrShadowHost,
} from '../../../neo-playwright/third_party/playwright/packages/injected/src/domUtils'
import type { AriaRole } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'

export type AccessibleName = {
  text: string
  elements?: Set<Element>
  derivedFromContent: boolean
}

export type AccessibleDescription = {
  text: string
  derivedFromContent: boolean
}

const ANCESTOR_PREVENTING_LANDMARK =
  'article:not([role]), aside:not([role]), main:not([role]), nav:not([role]), section:not([role]), [role=article], [role=complementary], [role=main], [role=navigation], [role=region]'

const VALID_ROLES: ReadonlySet<AriaRole> = new Set([
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
])

const ARIA_CHECKED_ROLES: ReadonlySet<string> = new Set([
  'checkbox',
  'menuitemcheckbox',
  'option',
  'radio',
  'switch',
  'menuitemradio',
  'treeitem',
])

const ARIA_READONLY_ROLES: ReadonlySet<string> = new Set([
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
])

const ARIA_DISABLED_ROLES: ReadonlySet<string> = new Set([
  'application',
  'button',
  'composite',
  'gridcell',
  'group',
  'input',
  'link',
  'menuitem',
  'scrollbar',
  'separator',
  'tab',
  'checkbox',
  'columnheader',
  'combobox',
  'grid',
  'listbox',
  'menu',
  'menubar',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'radio',
  'radiogroup',
  'row',
  'rowheader',
  'searchbox',
  'select',
  'slider',
  'spinbutton',
  'switch',
  'tablist',
  'textbox',
  'toolbar',
  'tree',
  'treegrid',
  'treeitem',
])

const LABEL_PROHIBITED_ROLES: ReadonlySet<string> = new Set([
  'caption',
  'code',
  'deletion',
  'emphasis',
  'generic',
  'insertion',
  'paragraph',
  'presentation',
  'strong',
  'subscript',
  'superscript',
])

const ROLEDESCRIPTION_PROHIBITED_ROLES: ReadonlySet<string> = new Set([
  'generic',
])

const GLOBAL_ARIA_ATTRIBUTES: readonly (readonly [
  string,
  ReadonlySet<string> | undefined,
])[] = [
  ['aria-atomic', undefined],
  ['aria-busy', undefined],
  ['aria-controls', undefined],
  ['aria-current', undefined],
  ['aria-describedby', undefined],
  ['aria-details', undefined],
  ['aria-dropeffect', undefined],
  ['aria-flowto', undefined],
  ['aria-grabbed', undefined],
  ['aria-hidden', undefined],
  ['aria-keyshortcuts', undefined],
  ['aria-label', LABEL_PROHIBITED_ROLES],
  ['aria-labelledby', LABEL_PROHIBITED_ROLES],
  ['aria-live', undefined],
  ['aria-owns', undefined],
  ['aria-relevant', undefined],
  ['aria-roledescription', ROLEDESCRIPTION_PROHIBITED_ROLES],
]

const PRESENTATION_INHERITANCE_PARENTS: Readonly<
  Record<string, ReadonlySet<string>>
> = {
  DD: new Set(['DL', 'DIV']),
  DIV: new Set(['DL']),
  DT: new Set(['DL', 'DIV']),
  LI: new Set(['OL', 'UL']),
  TBODY: new Set(['TABLE']),
  TD: new Set(['TR']),
  TFOOT: new Set(['TABLE']),
  TH: new Set(['TR']),
  THEAD: new Set(['TABLE']),
  TR: new Set(['THEAD', 'TBODY', 'TFOOT', 'TABLE']),
}

const INPUT_TYPE_TO_ROLE: Readonly<Record<string, AriaRole>> = {
  button: 'button',
  checkbox: 'checkbox',
  image: 'button',
  number: 'spinbutton',
  radio: 'radio',
  range: 'slider',
  reset: 'button',
  submit: 'button',
}

const TEXT_LIKE_INPUT_TYPES: ReadonlySet<string> = new Set([
  'email',
  'search',
  'tel',
  'text',
  'url',
  '',
])

const NATIVELY_FOCUSABLE_TAGS: ReadonlySet<string> = new Set([
  'BUTTON',
  'DETAILS',
  'SELECT',
  'TEXTAREA',
])

const NATIVE_FORM_CONTROL_TAGS: ReadonlySet<string> = new Set([
  'BUTTON',
  'INPUT',
  'SELECT',
  'TEXTAREA',
  'OPTION',
  'OPTGROUP',
])

const NATIVE_READONLY_TAGS: ReadonlySet<string> = new Set([
  'INPUT',
  'TEXTAREA',
  'SELECT',
])

const STATIC_IMPLICIT_ROLES: Readonly<Record<string, AriaRole>> = {
  ARTICLE: 'article',
  ASIDE: 'complementary',
  BLOCKQUOTE: 'blockquote',
  BUTTON: 'button',
  CAPTION: 'caption',
  CODE: 'code',
  DATALIST: 'listbox',
  DD: 'definition',
  DEL: 'deletion',
  DETAILS: 'group',
  DFN: 'term',
  DIALOG: 'dialog',
  DT: 'term',
  EM: 'emphasis',
  FIELDSET: 'group',
  FIGURE: 'figure',
  H1: 'heading',
  H2: 'heading',
  H3: 'heading',
  H4: 'heading',
  H5: 'heading',
  H6: 'heading',
  HR: 'separator',
  HTML: 'document',
  INS: 'insertion',
  LI: 'listitem',
  MAIN: 'main',
  MARK: 'mark',
  MATH: 'math',
  MENU: 'list',
  METER: 'meter',
  NAV: 'navigation',
  OL: 'list',
  OPTGROUP: 'group',
  OPTION: 'option',
  OUTPUT: 'status',
  P: 'paragraph',
  PROGRESS: 'progressbar',
  SEARCH: 'search',
  STRONG: 'strong',
  SUB: 'subscript',
  SUP: 'superscript',
  SVG: 'img',
  TABLE: 'table',
  TBODY: 'rowgroup',
  TEXTAREA: 'textbox',
  TFOOT: 'rowgroup',
  THEAD: 'rowgroup',
  TIME: 'time',
  TR: 'row',
  UL: 'list',
}

const DYNAMIC_IMPLICIT_ROLES: Readonly<
  Record<string, (element: Element) => AriaRole | null>
> = {
  A: (element) => (element.hasAttribute('href') ? 'link' : null),
  AREA: (element) => (element.hasAttribute('href') ? 'link' : null),
  FOOTER: (element) =>
    closestCrossShadow(element, ANCESTOR_PREVENTING_LANDMARK)
      ? null
      : 'contentinfo',
  FORM: (element) => (hasExplicitAccessibleName(element) ? 'form' : null),
  HEADER: (element) =>
    closestCrossShadow(element, ANCESTOR_PREVENTING_LANDMARK) ? null : 'banner',
  IMG: (element) =>
    element.getAttribute('alt') === '' &&
    !element.getAttribute('title') &&
    !hasGlobalAriaAttribute(element) &&
    !hasTabIndex(element)
      ? 'presentation'
      : 'img',
  INPUT: (element) => {
    const type = (element as HTMLInputElement).type.toLowerCase()
    if (TEXT_LIKE_INPUT_TYPES.has(type)) {
      const list = getIdRefs(element, element.getAttribute('list'))[0]
      if (list && elementSafeTagName(list) === 'DATALIST') return 'combobox'
      return type === 'search' ? 'searchbox' : 'textbox'
    }
    if (type === 'hidden') return null
    if (type === 'file') return 'button'
    return (
      (Object.hasOwn(INPUT_TYPE_TO_ROLE, type)
        ? INPUT_TYPE_TO_ROLE[type]
        : undefined) ?? 'textbox'
    )
  },
  SECTION: (element) => (hasExplicitAccessibleName(element) ? 'region' : null),
  SELECT: (element) =>
    isListBoxSelect(element as HTMLSelectElement) ? 'listbox' : 'combobox',
  TD: (element) => {
    const table = closestCrossShadow(element, 'table')
    const role = table ? getExplicitAriaRole(table) : ''
    return role === 'grid' || role === 'treegrid' ? 'gridcell' : 'cell'
  },
  TH: (element) => {
    const scope = element.getAttribute('scope')
    if (scope === 'col' || scope === 'colgroup') return 'columnheader'
    if (scope === 'row' || scope === 'rowgroup') return 'rowheader'

    const nextSibling = element.nextElementSibling
    const prevSibling = element.previousElementSibling
    const row =
      element.parentElement &&
      elementSafeTagName(element.parentElement) === 'TR'
        ? element.parentElement
        : undefined

    if (!nextSibling && !prevSibling) {
      if (row) {
        const table = closestCrossShadow(row, 'table') as
          | HTMLTableElement
          | undefined
        if (table && table.rows.length <= 1) return null
      }
      return 'columnheader'
    }

    if (isHeaderCell(nextSibling) && isHeaderCell(prevSibling))
      return 'columnheader'

    if (isNonEmptyDataCell(nextSibling) || isNonEmptyDataCell(prevSibling))
      return 'rowheader'

    return 'columnheader'
  },
}

function hasExplicitAccessibleName(element: Element): boolean {
  return (
    element.hasAttribute('aria-label') ||
    element.hasAttribute('aria-labelledby')
  )
}

function hasGlobalAriaAttribute(
  element: Element,
  forRole?: string | null,
): boolean {
  const role = forRole || ''
  return GLOBAL_ARIA_ATTRIBUTES.some(
    ([attr, prohibited]) =>
      !prohibited?.has(role) && element.hasAttribute(attr),
  )
}

function hasTabIndex(element: Element): boolean {
  return !Number.isNaN(Number(String(element.getAttribute('tabindex'))))
}

function isNativelyFocusable(element: Element): boolean {
  const tagName = elementSafeTagName(element)
  if (NATIVELY_FOCUSABLE_TAGS.has(tagName)) return true
  if (tagName === 'A' || tagName === 'AREA') return element.hasAttribute('href')
  if (tagName === 'INPUT') return !(element as HTMLInputElement).hidden
  return false
}

function isFocusable(element: Element): boolean {
  return (
    !isNativelyDisabled(element) &&
    (isNativelyFocusable(element) || hasTabIndex(element))
  )
}

function isHeaderCell(element: Element | null): boolean {
  return !!element && elementSafeTagName(element) === 'TH'
}

function isNonEmptyDataCell(element: Element | null): boolean {
  if (!element || elementSafeTagName(element) !== 'TD') return false
  return !!(element.textContent?.trim() || element.children.length > 0)
}

function getExplicitAriaRole(element: Element): AriaRole | null {
  const roles = (element.getAttribute('role') || '')
    .split(' ')
    .map((role) => role.trim())
  return (
    (roles.find((role) => VALID_ROLES.has(role as AriaRole)) as AriaRole) ||
    null
  )
}

function hasPresentationConflictResolution(
  element: Element,
  role: string | null,
): boolean {
  return hasGlobalAriaAttribute(element, role) || isFocusable(element)
}

function lookupImplicitRole(element: Element): AriaRole | null {
  const tagName = elementSafeTagName(element)
  if (Object.hasOwn(STATIC_IMPLICIT_ROLES, tagName))
    return STATIC_IMPLICIT_ROLES[tagName] ?? null
  if (Object.hasOwn(DYNAMIC_IMPLICIT_ROLES, tagName))
    return DYNAMIC_IMPLICIT_ROLES[tagName]?.(element) ?? null
  return null
}

function getImplicitAriaRole(element: Element): AriaRole | null {
  const implicitRole = lookupImplicitRole(element)
  if (!implicitRole) return null
  let ancestor: Element | null = element
  while (ancestor) {
    const parent = parentElementOrShadowHost(ancestor)
    const ancestorTag = elementSafeTagName(ancestor)
    const parents = Object.hasOwn(PRESENTATION_INHERITANCE_PARENTS, ancestorTag)
      ? PRESENTATION_INHERITANCE_PARENTS[ancestorTag]
      : undefined
    if (!parents || !parent || !parents.has(elementSafeTagName(parent))) break
    const parentExplicitRole = getExplicitAriaRole(parent)
    if (
      (parentExplicitRole === 'none' ||
        parentExplicitRole === 'presentation') &&
      !hasPresentationConflictResolution(parent, parentExplicitRole)
    )
      return parentExplicitRole
    ancestor = parent
  }
  return implicitRole
}

function getIdRefs(element: Element, ref: string | null): Element[] {
  if (!ref) return []
  const root = enclosingShadowRootOrDocument(element)
  if (!root) return []
  try {
    const ids = ref.split(' ').filter((id) => !!id)
    const result: Element[] = []
    for (const id of ids) {
      const firstElement = root.querySelector(`#${CSS.escape(id)}`)
      if (firstElement && !result.includes(firstElement))
        result.push(firstElement)
    }
    return result
  } catch {
    return []
  }
}

function belongsToDisabledOptGroup(element: Element): boolean {
  return (
    elementSafeTagName(element) === 'OPTION' &&
    !!element.closest('OPTGROUP[DISABLED]')
  )
}

function belongsToDisabledFieldSet(element: Element): boolean {
  const fieldSetElement = element?.closest('FIELDSET[DISABLED]')
  if (!fieldSetElement) return false
  const legendElement = fieldSetElement.querySelector(':scope > LEGEND')
  return !legendElement?.contains(element)
}

function isNativelyDisabled(element: Element): boolean {
  return (
    NATIVE_FORM_CONTROL_TAGS.has(elementSafeTagName(element)) &&
    (element.hasAttribute('disabled') ||
      belongsToDisabledOptGroup(element) ||
      belongsToDisabledFieldSet(element))
  )
}

function hasAriaDisabledInChain(element: Element): boolean {
  let current: Element | undefined = element
  while (current) {
    const attribute = (
      current.getAttribute('aria-disabled') || ''
    ).toLowerCase()
    if (attribute === 'true') return true
    if (attribute === 'false') return false
    current = parentElementOrShadowHost(current)
  }
  return false
}

function hasExplicitAriaDisabled(element: Element): boolean {
  if (!ARIA_DISABLED_ROLES.has(computeAriaRole(element) || '')) return false
  return hasAriaDisabledInChain(element)
}

function getChecked(
  element: Element,
  allowMixed: boolean,
): boolean | 'mixed' | 'error' {
  const tagName = elementSafeTagName(element)
  if (
    allowMixed &&
    tagName === 'INPUT' &&
    (element as HTMLInputElement).indeterminate
  )
    return 'mixed'
  if (
    tagName === 'INPUT' &&
    ((element as HTMLInputElement).type === 'checkbox' ||
      (element as HTMLInputElement).type === 'radio')
  )
    return (element as HTMLInputElement).checked
  if (ARIA_CHECKED_ROLES.has(computeAriaRole(element) || '')) {
    const checked = element.getAttribute('aria-checked')
    if (checked === 'true') return true
    if (allowMixed && checked === 'mixed') return 'mixed'
    return false
  }
  return 'error'
}

function unsupportedInPage(name: string): never {
  throw new Error(
    `${name}: accessible name, description, and error message computation runs in Rust, not in the page`,
  )
}

export function computeAriaRole(element: Element): AriaRole | null {
  const explicitRole = getExplicitAriaRole(element)
  if (!explicitRole) return getImplicitAriaRole(element)
  if (explicitRole === 'none' || explicitRole === 'presentation') {
    const implicitRole = getImplicitAriaRole(element)
    if (hasPresentationConflictResolution(element, implicitRole))
      return implicitRole
  }
  return explicitRole
}

/**
 * Intentionally returns `null` so vendored `selectorGenerator.ts` skips role candidates
 * without calling the throwing accessible-name functions.
 */
export function getAriaRole(_element: Element): AriaRole | null {
  return null
}

export function getAriaDisabled(element: Element): boolean {
  return isNativelyDisabled(element) || hasExplicitAriaDisabled(element)
}

export function getReadonly(element: Element): boolean | 'error' {
  const tagName = elementSafeTagName(element)
  if (NATIVE_READONLY_TAGS.has(tagName)) return element.hasAttribute('readonly')
  if (ARIA_READONLY_ROLES.has(computeAriaRole(element) || ''))
    return element.getAttribute('aria-readonly') === 'true'
  if ((element as HTMLElement).isContentEditable) return false
  return 'error'
}

export function getCheckedAllowMixed(
  element: Element,
): boolean | 'mixed' | 'error' {
  return getChecked(element, true)
}

export function getCheckedWithoutMixed(element: Element): boolean | 'error' {
  return getChecked(element, false) as boolean | 'error'
}

export function getAriaLabelledByElements(element: Element): Element[] | null {
  const ref = element.getAttribute('aria-labelledby')
  if (ref === null) return null
  const refs = getIdRefs(element, ref)
  return refs.length ? refs : null
}

export function beginAriaCaches(): void {}

export function endAriaCaches(): void {}

export function getElementAccessibleName(
  _element: Element,
  _includeHidden: boolean,
): AccessibleName {
  return unsupportedInPage('getElementAccessibleName')
}

export function getElementAccessibleNameText(
  _element: Element,
  _includeHidden: boolean,
): string {
  return unsupportedInPage('getElementAccessibleNameText')
}

export function getElementAccessibleDescription(
  _element: Element,
  _includeHidden: boolean,
): AccessibleDescription {
  return unsupportedInPage('getElementAccessibleDescription')
}

export function getElementAccessibleErrorMessage(_element: Element): string {
  return unsupportedInPage('getElementAccessibleErrorMessage')
}
