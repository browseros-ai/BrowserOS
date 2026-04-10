import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

interface AXValue {
  type: string
  value?: string | number | boolean
}

interface AXProperty {
  name: string
  value: AXValue
}

export interface AXNode {
  nodeId: string
  ignored?: boolean
  role?: AXValue
  name?: AXValue
  description?: AXValue
  value?: AXValue
  properties?: AXProperty[]
  childIds?: string[]
  backendDOMNodeId?: number
}

const INTERACTIVE_ROLES = new Set([
  'button',
  'link',
  'textbox',
  'searchbox',
  'textarea',
  'checkbox',
  'radio',
  'combobox',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'tab',
  'switch',
  'slider',
  'spinbutton',
  'option',
  'treeitem',
  'listbox',
  'DisclosureTriangle',
])

const NAMED_CONTENT_ROLES = new Set([
  'heading',
  'img',
  'cell',
  'columnheader',
  'rowheader',
  'dialog',
  'alertdialog',
])

const SKIP_ROLES = new Set([
  'none',
  'presentation',
  'LineBreak',
  'InlineTextBox',
])

export function buildInteractiveTree(nodes: AXNode[]): string[] {
  const nodeMap = new Map<string, AXNode>()
  for (const node of nodes) nodeMap.set(node.nodeId, node)

  const lines: string[] = []

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: tree-walking with multiple node types is inherently complex
  function walk(nodeId: string): void {
    const node = nodeMap.get(nodeId)
    if (!node) return

    const role = node.ignored
      ? undefined
      : (node.role?.value as string | undefined)
    if (!role || SKIP_ROLES.has(role)) {
      if (node.childIds) for (const childId of node.childIds) walk(childId)
      return
    }

    if (INTERACTIVE_ROLES.has(role) && node.backendDOMNodeId !== undefined) {
      const name = typeof node.name?.value === 'string' ? node.name.value : ''
      const value =
        typeof node.value?.value === 'string' ? node.value.value : ''

      let line = `[${node.backendDOMNodeId}] ${role}`
      if (name) line += ` "${name}"`
      if (
        value &&
        (role === 'textbox' || role === 'searchbox' || role === 'textarea')
      )
        line += ` value="${value}"`
      const props = extractProps(node)
      if (props) line += ` ${props}`
      lines.push(line)
    }

    if (node.childIds) for (const childId of node.childIds) walk(childId)
  }

  const roots = nodes.filter(
    (n) => n.role?.value === 'RootWebArea' || n.role?.value === 'WebArea',
  )
  if (roots.length === 0 && nodes[0]?.childIds) {
    for (const childId of nodes[0].childIds) walk(childId)
  } else {
    for (const root of roots) {
      if (root.childIds) for (const childId of root.childIds) walk(childId)
    }
  }

  return lines
}

export function buildEnhancedTree(nodes: AXNode[]): string[] {
  const nodeMap = new Map<string, AXNode>()
  for (const node of nodes) nodeMap.set(node.nodeId, node)

  const lines: string[] = []

  // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: tree-walking with multiple node types is inherently complex
  function walk(nodeId: string, depth: number): void {
    const node = nodeMap.get(nodeId)
    if (!node) return

    const role = node.ignored
      ? undefined
      : (node.role?.value as string | undefined)
    if (!role || SKIP_ROLES.has(role)) {
      if (node.childIds)
        for (const childId of node.childIds) walk(childId, depth)
      return
    }

    const name = typeof node.name?.value === 'string' ? node.name.value : ''
    const value = typeof node.value?.value === 'string' ? node.value.value : ''
    const isInteractive = INTERACTIVE_ROLES.has(role)
    const isNamedContent = NAMED_CONTENT_ROLES.has(role) && name.length > 0
    const hasId =
      (isInteractive || isNamedContent) && node.backendDOMNodeId !== undefined

    const indent = '  '.repeat(depth)
    let line: string

    if (hasId) {
      line = `${indent}[${node.backendDOMNodeId}] ${role}`
    } else {
      line = `${indent}- ${role}`
    }

    if (name) line += ` "${name}"`
    if (
      value &&
      (role === 'textbox' || role === 'searchbox' || role === 'textarea')
    )
      line += ` value="${value}"`
    const props = extractProps(node)
    if (props) line += ` ${props}`

    lines.push(line)

    if (node.childIds)
      for (const childId of node.childIds) walk(childId, depth + 1)
  }

  const roots = nodes.filter(
    (n) => n.role?.value === 'RootWebArea' || n.role?.value === 'WebArea',
  )
  if (roots.length === 0 && nodes[0]?.childIds) {
    for (const childId of nodes[0].childIds) walk(childId, 0)
  } else {
    for (const root of roots) {
      if (root.childIds) for (const childId of root.childIds) walk(childId, 0)
    }
  }

  return lines
}

const CURSOR_INTERACTIVE_JS = `(function() {
	var interactiveRoles = new Set([
		'button','link','textbox','checkbox','radio','combobox','listbox',
		'menuitem','menuitemcheckbox','menuitemradio','option','searchbox',
		'slider','spinbutton','switch','tab','treeitem'
	]);
	var interactiveTags = new Set([
		'a','button','input','select','textarea','details','summary'
	]);
	var results = [];
	var allElements = document.body.querySelectorAll('*');
	for (var i = 0; i < allElements.length; i++) {
		var el = allElements[i];
		var tag = el.tagName.toLowerCase();
		if (interactiveTags.has(tag)) continue;
		var role = el.getAttribute('role');
		if (role && interactiveRoles.has(role.toLowerCase())) continue;
		var style = getComputedStyle(el);
		var hasCursor = style.cursor === 'pointer';
		var hasOnClick = el.hasAttribute('onclick') || el.onclick !== null;
		var tabIdx = el.getAttribute('tabindex');
		var hasTabIndex = tabIdx !== null && tabIdx !== '-1';
		if (!hasCursor && !hasOnClick && !hasTabIndex) continue;
		if (hasCursor && !hasOnClick && !hasTabIndex) {
			var parent = el.parentElement;
			if (parent && getComputedStyle(parent).cursor === 'pointer') continue;
		}
		var text = (el.textContent || '').trim().slice(0, 100);
		if (!text) text = (el.getAttribute('aria-label') || '').trim();
		if (!text) continue;
		var rect = el.getBoundingClientRect();
		if (rect.width === 0 || rect.height === 0) continue;
		el.setAttribute('data-__cid', String(i));
		var reasons = [];
		if (hasCursor) reasons.push('cursor:pointer');
		if (hasOnClick) reasons.push('onclick');
		if (hasTabIndex) reasons.push('tabindex');
		results.push({ marker: String(i), text: text, reasons: reasons });
	}
	return results;
})()`

export interface CursorInteractiveElement {
  backendNodeId: number
  text: string
  reasons: string[]
}

export async function findCursorInteractiveElements(
  session: ProtocolApi,
): Promise<CursorInteractiveElement[]> {
  const findResult = await session.Runtime.evaluate({
    expression: CURSOR_INTERACTIVE_JS,
    returnByValue: true,
  })

  const found = findResult.result?.value as
    | Array<{ marker: string; text: string; reasons: string[] }>
    | undefined
  if (!found?.length) return []

  const results: CursorInteractiveElement[] = []

  for (const el of found) {
    try {
      const queryResult = await session.Runtime.evaluate({
        expression: `document.querySelector('[data-__cid="${el.marker}"]')`,
        returnByValue: false,
      })

      if (!queryResult.result?.objectId) continue

      const desc = await session.DOM.describeNode({
        objectId: queryResult.result.objectId,
      })

      if (desc.node?.backendNodeId) {
        results.push({
          backendNodeId: desc.node.backendNodeId,
          text: el.text,
          reasons: el.reasons,
        })
      }
    } catch {
      // skip unresolvable elements
    }
  }

  await session.Runtime.evaluate({
    expression: `document.querySelectorAll('[data-__cid]').forEach(function(el){el.removeAttribute('data-__cid')})`,
    returnByValue: true,
  })

  return results
}

export interface LinkNode {
  backendDOMNodeId: number
  text: string
}

export function extractLinkNodes(nodes: AXNode[]): LinkNode[] {
  const nodeMap = new Map<string, AXNode>()
  for (const node of nodes) nodeMap.set(node.nodeId, node)

  const links: LinkNode[] = []

  function walk(nodeId: string): void {
    const node = nodeMap.get(nodeId)
    if (!node) return

    const role = node.ignored
      ? undefined
      : (node.role?.value as string | undefined)

    if (role === 'link' && node.backendDOMNodeId !== undefined) {
      const text = typeof node.name?.value === 'string' ? node.name.value : ''
      links.push({ backendDOMNodeId: node.backendDOMNodeId, text })
    }

    if (node.childIds) for (const childId of node.childIds) walk(childId)
  }

  const roots = nodes.filter(
    (n) => n.role?.value === 'RootWebArea' || n.role?.value === 'WebArea',
  )
  if (roots.length === 0 && nodes[0]?.childIds) {
    for (const childId of nodes[0].childIds) walk(childId)
  } else {
    for (const root of roots) {
      if (root.childIds) for (const childId of root.childIds) walk(childId)
    }
  }

  return links
}

function extractProps(node: AXNode): string {
  const parts: string[] = []
  if (!node.properties) return ''

  for (const prop of node.properties) {
    if (prop.name === 'checked' && prop.value.value === true)
      parts.push('checked')
    if (prop.name === 'checked' && prop.value.value === 'mixed')
      parts.push('indeterminate')
    if (prop.name === 'disabled' && prop.value.value === true)
      parts.push('disabled')
    if (prop.name === 'expanded' && prop.value.value === true)
      parts.push('expanded')
    if (prop.name === 'expanded' && prop.value.value === false)
      parts.push('collapsed')
    if (prop.name === 'required' && prop.value.value === true)
      parts.push('required')
    if (prop.name === 'selected' && prop.value.value === true)
      parts.push('selected')
    if (prop.name === 'level') parts.push(`level=${prop.value.value}`)
  }

  return parts.length > 0 ? `(${parts.join(', ')})` : ''
}
