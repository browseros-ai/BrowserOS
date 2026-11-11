/**
 * The tree and token helpers `code-gaps.ts` decides with: a walk over acorn's nodes, the token
 * and comment items in source order, and lookups over them by offset.
 */

import type { Node } from 'acorn'

export interface Item {
  start: number
  end: number
  text: string
  kind: 'token' | 'line-comment' | 'block-comment'
  /** For tokens: acorn's keyword name, when the token is one. */
  keyword: string | null
}

export type AnyNode = Node & Record<string, unknown>

/** The statements a node lays out one per line. */
export function statementList(node: AnyNode): AnyNode[] | null {
  switch (node.type) {
    case 'Program':
    case 'BlockStatement':
    case 'StaticBlock':
    case 'ClassBody':
      return node.body as AnyNode[]
    case 'SwitchStatement':
      return node.cases as AnyNode[]
    case 'SwitchCase':
      return node.consequent as AnyNode[]
    default:
      return null
  }
}

export function isCallLike(node: AnyNode): boolean {
  return node.type === 'CallExpression' || node.type === 'NewExpression'
}

/** Whether `child` is an inner link of the chain `parent` continues (`a.b()` inside `a.b().c()`). */
export function isChainLink(parent: AnyNode, child: AnyNode): boolean {
  return (
    (parent.type === 'MemberExpression' && parent.object === child) ||
    (isCallLike(parent) && parent.callee === child)
  )
}

/** The indentation the author used (the smallest one), or 2. */
export function indentUnit(code: string): number {
  let unit = 0
  for (const line of code.split('\n')) {
    const leading = /^( +)\S/.exec(line)?.[1]?.length ?? 0
    if (leading > 0 && (unit === 0 || leading < unit)) unit = leading
  }
  return unit >= 2 && unit <= 8 ? unit : 2
}

export function walk(
  root: AnyNode,
  visit: (node: AnyNode, parent: AnyNode | null) => void,
) {
  const stack: [AnyNode, AnyNode | null][] = [[root, null]]
  while (stack.length > 0) {
    const [node, parent] = stack.pop() as [AnyNode, AnyNode | null]
    visit(node, parent)
    for (const key of Object.keys(node)) {
      const value = node[key]
      if (Array.isArray(value)) {
        for (let i = value.length - 1; i >= 0; i--) {
          const child = value[i]
          if (isNode(child)) stack.push([child, node])
        }
      } else if (isNode(value)) {
        stack.push([value, node])
      }
    }
  }
}

export function isNode(value: unknown): value is AnyNode {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as { type?: unknown }).type === 'string' &&
    typeof (value as { start?: unknown }).start === 'number'
  )
}

/** The index of the first item starting at or after `offset`. */
export function lowerBound(items: Item[], offset: number): number {
  let low = 0
  let high = items.length
  while (low < high) {
    const mid = (low + high) >> 1
    if ((items[mid] as Item).start < offset) low = mid + 1
    else high = mid
  }
  return low
}

/** The last token with this text in [from, to). */
export function lastBefore(
  items: Item[],
  text: string,
  from: number,
  to: number,
): number {
  for (let i = lowerBound(items, to) - 1; i >= 0; i--) {
    const item = items[i] as Item
    if (item.start < from) break
    if (item.kind === 'token' && item.text === text) return i
  }
  return -1
}
