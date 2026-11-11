/**
 * The first half of `code-format.ts`: parse a run script and decide each gap between its tokens.
 *
 * acorn gives the tokens, the comments and the tree. Walking the tree decides, gap by gap, a break
 * (a new line indented relative to an anchor token, so nesting follows the line the anchor ended
 * up on) or a space; constructs that may break later, if their line is too wide, are collected as
 * candidates. Nothing here prints: `code-format.ts` lays out the decisions.
 */

import { type Comment, parse, type Token, tokenizer } from 'acorn'
import {
  type AnyNode,
  type Item,
  indentUnit,
  isCallLike,
  isChainLink,
  lastBefore,
  lowerBound,
  statementList,
  walk,
} from './code-tree'

const PARSE_OPTIONS = {
  ecmaVersion: 'latest',
  sourceType: 'script',
  // A run() script is an async function body: top-level await and return are legal.
  allowAwaitOutsideFunction: true,
  allowReturnOutsideFunction: true,
} as const

export type Decision =
  | { kind: 'space' }
  /** A new line, indented `extra` units past the line that holds the anchor item (or at 0). */
  | { kind: 'break'; anchor: number | null; extra: number }

export interface Analysis {
  program: AnyNode & { body: AnyNode[] }
  items: Item[]
  decisions: Map<number, Decision>
  /** Constructs a too-wide line may still break. */
  candidates: Candidate[]
  unit: number
}

export interface Candidate {
  /** 0 callback body, 1 object or array literal, 2 method chain, 3 call arguments. */
  rank: number
  /** The item the construct starts at: it is broken when that item's line is too wide. */
  first: number
  /** How much text breaking it moves off the line: a too-wide line breaks its largest first. */
  size: number
  breaks: [number, Decision][]
}

export function analyse(code: string): Analysis | null {
  const comments: Comment[] = []
  let program: AnyNode & { body: AnyNode[] }
  const tokens: Token[] = []
  try {
    program = parse(code, {
      ...PARSE_OPTIONS,
      onComment: comments,
    }) as unknown as AnyNode & { body: AnyNode[] }
    for (const token of tokenizer(code, PARSE_OPTIONS)) tokens.push(token)
  } catch {
    return null
  }
  const gaps = new Gaps(code, tokens, comments)
  // Template literals first, so nothing is decided inside them.
  walk(program, (node) => {
    if (node.type === 'TemplateLiteral')
      gaps.templates.push([node.start, node.end])
  })
  walk(program, (node, parent) => {
    breakStatements(node, gaps)
    RULES[node.type]?.(node, parent, gaps)
  })
  spaceTokens(gaps)
  return {
    program,
    items: gaps.items,
    decisions: gaps.decisions,
    candidates: gaps.candidates.filter(
      (candidate) =>
        candidate.first >= 0 && candidate.breaks.every(([index]) => index >= 0),
    ),
    unit: indentUnit(code),
  }
}

/** The script's tokens and comments, and what has been decided about the gaps between them. */
class Gaps {
  readonly items: Item[]
  readonly decisions = new Map<number, Decision>()
  readonly candidates: Candidate[] = []
  readonly templates: [number, number][] = []
  private readonly byStart: Map<number, number>

  constructor(code: string, tokens: Token[], comments: Comment[]) {
    this.items = [
      ...tokens.map(
        (token): Item => ({
          start: token.start,
          end: token.end,
          text: code.slice(token.start, token.end),
          kind: 'token',
          keyword: token.type.keyword ?? null,
        }),
      ),
      ...comments.map(
        (comment): Item => ({
          start: comment.start,
          end: comment.end,
          text: code.slice(comment.start, comment.end),
          kind: comment.type === 'Line' ? 'line-comment' : 'block-comment',
          keyword: null,
        }),
      ),
    ].sort((a, b) => a.start - b.start)
    this.byStart = new Map(this.items.map((item, index) => [item.start, index]))
  }

  /** The item that starts at this offset (every node starts at one), or -1. */
  at(offset: number): number {
    return this.byStart.get(offset) ?? -1
  }

  /** The first token with this text in [from, to), or -1. */
  find(text: string, from: number, to: number): number {
    for (let i = lowerBound(this.items, from); i < this.items.length; i++) {
      const item = this.items[i] as Item
      if (item.start >= to) break
      if (item.kind === 'token' && item.text === text) return i
    }
    return -1
  }

  /** Decide the gap before item `index`; a break is never weakened to a space. */
  decide(index: number, decision: Decision): void {
    if (index <= 0 || index >= this.items.length) return
    const start = (this.items[index] as Item).start
    // Inside a template literal nothing moves: not even the code in `${}`.
    if (this.templates.some(([from, to]) => start > from && start < to)) return
    if (
      this.decisions.get(index)?.kind === 'break' &&
      decision.kind === 'space'
    ) {
      return
    }
    this.decisions.set(index, decision)
  }

  space(index: number): void {
    this.decide(index, { kind: 'space' })
  }

  spaceAround(index: number): void {
    if (index < 0) return
    this.space(index)
    this.space(index + 1)
  }

  breakAt(index: number, anchor: number, extra: number): [number, Decision] {
    return [index, { kind: 'break', anchor, extra }]
  }
}

/** Each statement of a body on its own line, one unit past its opening brace; the brace closes alone. */
function breakStatements(node: AnyNode, gaps: Gaps): void {
  const list = statementList(node)
  if (!list) return
  const open =
    node.type === 'Program'
      ? null
      : node.type === 'SwitchStatement'
        ? gaps.find('{', (node.discriminant as AnyNode).end, node.end)
        : gaps.at(node.start)
  for (const statement of list) {
    if (statement.type === 'EmptyStatement') continue
    gaps.decide(gaps.at(statement.start), {
      kind: 'break',
      anchor: open,
      extra: open === null ? 0 : 1,
    })
  }
  const closes =
    node.type !== 'Program' && node.type !== 'SwitchCase' && list.length > 0
  if (closes && open !== null && open >= 0) {
    gaps.decide(gaps.at(node.end - 1), {
      kind: 'break',
      anchor: open,
      extra: 0,
    })
  }
}

type Rule = (node: AnyNode, parent: AnyNode | null, gaps: Gaps) => void

const spaceOperator: Rule = (node, _parent, gaps) => {
  const left = node.left as AnyNode
  const right = node.right as AnyNode
  gaps.spaceAround(gaps.find(node.operator as string, left.end, right.start))
}

/** Members one per line, for a literal on a line that is too wide. */
const literalMembers: Rule = (node, _parent, gaps) => {
  const members = (
    node.type === 'ArrayExpression' ? node.elements : node.properties
  ) as (AnyNode | null)[]
  if (members.length === 0) return
  const open = gaps.at(node.start)
  const close = gaps.at(node.end - 1)
  if (node.type !== 'ArrayExpression') {
    gaps.space(open + 1)
    gaps.space(close)
  }
  const breaks = members
    .filter((member): member is AnyNode => member !== null)
    .map((member) => gaps.breakAt(gaps.at(member.start), open, 1))
  breaks.push(gaps.breakAt(close, open, 0))
  gaps.candidates.push({
    rank: 1,
    first: open,
    size: node.end - node.start - 2,
    breaks,
  })
}

/** A chain before each `.` after its first; then the arguments one per line. */
const callBreaks: Rule = (node, parent, gaps) => {
  const dots = chainDots(node, gaps)
  if (dots.length >= 2 && !(parent && isChainLink(parent, node))) {
    const head = gaps.at(node.start)
    gaps.candidates.push({
      rank: 2,
      first: head,
      size: node.end - (gaps.items[dots[1] as number] as Item).start,
      breaks: dots.slice(1).map((dot) => gaps.breakAt(dot, head, 1)),
    })
  }
  const args = node.arguments as AnyNode[]
  const callee = node.callee as AnyNode
  const open = gaps.find('(', callee.end, args[0]?.start ?? node.end)
  if (args.length < 2 || open < 0) return
  const breaks = args.map((arg) => gaps.breakAt(gaps.at(arg.start), open, 1))
  breaks.push(gaps.breakAt(gaps.at(node.end - 1), open, 0))
  gaps.candidates.push({
    rank: 3,
    first: open,
    size: (args.at(-1) as AnyNode).end - (args[0] as AnyNode).start,
    breaks,
  })
}

const RULES: Record<string, Rule> = {
  BinaryExpression: spaceOperator,
  LogicalExpression: spaceOperator,
  AssignmentExpression: spaceOperator,
  AssignmentPattern: (node, _parent, gaps) =>
    gaps.spaceAround(
      gaps.find('=', (node.left as AnyNode).end, (node.right as AnyNode).start),
    ),
  VariableDeclarator: (node, _parent, gaps) => {
    if (!node.init) return
    gaps.spaceAround(
      gaps.find('=', (node.id as AnyNode).end, (node.init as AnyNode).start),
    )
  },
  ConditionalExpression: (node, _parent, gaps) => {
    const consequent = node.consequent as AnyNode
    gaps.spaceAround(
      gaps.find('?', (node.test as AnyNode).end, consequent.start),
    )
    gaps.spaceAround(
      gaps.find(':', consequent.end, (node.alternate as AnyNode).start),
    )
  },
  /**
   * `=>` spaced; a callback that is a call's last argument may take its expression body onto
   * its own indented line, with the call's `)` closing under the call (a block body already
   * breaks, by `breakStatements`).
   */
  ArrowFunctionExpression: (node, parent, gaps) => {
    const body = node.body as AnyNode
    const arrow = lastBefore(gaps.items, '=>', node.start, body.start)
    gaps.spaceAround(arrow)
    if (!parent || !isCallLike(parent) || body.type === 'BlockStatement') return
    if ((parent.arguments as AnyNode[]).at(-1) !== node || arrow < 0) return
    const head = gaps.at(parent.start)
    gaps.candidates.push({
      rank: 0,
      first: head,
      size: node.end - (gaps.items[arrow + 1] as Item).start,
      breaks: [
        gaps.breakAt(arrow + 1, arrow, 1),
        gaps.breakAt(gaps.at(parent.end - 1), head, 0),
      ],
    })
  },
  Property: (node, _parent, gaps) => {
    if (node.kind !== 'init' || node.shorthand || node.method) return
    const colon = gaps.find(
      ':',
      (node.key as AnyNode).end,
      (node.value as AnyNode).start,
    )
    if (colon >= 0) gaps.space(colon + 1)
  },
  ObjectExpression: literalMembers,
  ObjectPattern: literalMembers,
  ArrayExpression: literalMembers,
  CallExpression: callBreaks,
  NewExpression: callBreaks,
  /** `for (a; b; c)`: a space after each `;` of the header, which stays on one line. */
  ForStatement: (node, _parent, gaps) => {
    const body = node.body as AnyNode
    const { items } = gaps
    for (let i = lowerBound(items, node.start); i < items.length; i++) {
      const item = items[i] as Item
      if (item.start >= body.start) break
      const next = items[i + 1]
      if (
        item.kind === 'token' &&
        item.text === ';' &&
        next &&
        !/^[;)]$/.test(next.text)
      ) {
        gaps.space(i + 1)
      }
    }
  },
  BlockStatement: (node, _parent, gaps) => {
    const open = gaps.at(node.start)
    const before = gaps.items[open - 1]
    if (before && /^(\)|else|try|finally|do|=>)$/.test(before.text)) {
      gaps.space(open)
    }
  },
  ClassBody: (node, _parent, gaps) => gaps.space(gaps.at(node.start)),
}

const KEYWORD_BEFORE_PAREN = /^(if|for|while|switch|catch|with)$/

const AFTER_BRACE = /^(else|catch|finally|while)$/

const VALUE_KEYWORD = /^(this|super|null|true|false|import)$/

const CONTEXTUAL_KEYWORD = /^(await|of|async|yield)$/

const CLOSING = /^[;),.\]}:]$/

/** Token-level spacing that the tree does not name: commas, keywords, `} else`. */
function spaceTokens(gaps: Gaps): void {
  const { items } = gaps
  for (let i = 1; i < items.length; i++) {
    const item = items[i] as Item
    const previous = items[i - 1] as Item
    if (item.kind !== 'token' || previous.kind !== 'token') continue
    const member = /^\??\.$/.test(items[i - 2]?.text ?? '')
    const keywordBefore =
      !member &&
      ((previous.keyword !== null && !VALUE_KEYWORD.test(previous.keyword)) ||
        CONTEXTUAL_KEYWORD.test(previous.text))
    if (
      (previous.text === ',' && !/^[,)\]}]$/.test(item.text)) ||
      (item.text === '(' &&
        KEYWORD_BEFORE_PAREN.test(previous.text) &&
        !member) ||
      (AFTER_BRACE.test(item.text) && previous.text === '}') ||
      (keywordBefore && !CLOSING.test(item.text)) ||
      (previous.text === ')' && item.keyword !== null && item.keyword !== 'in')
    ) {
      gaps.space(i)
    }
  }
}

/** The `.` (or `?.`) tokens of a call chain, in order: `a.b().c().d()` has three. */
function chainDots(node: AnyNode, gaps: Gaps): number[] {
  const dots: number[] = []
  let current: AnyNode | undefined = node
  while (current) {
    if (isCallLike(current)) {
      current = current.callee as AnyNode
    } else if (current.type === 'MemberExpression' && !current.computed) {
      const object = current.object as AnyNode
      const property = current.property as AnyNode
      const dot = gaps.find('.', object.end, property.start)
      const found = dot >= 0 ? dot : gaps.find('?.', object.end, property.start)
      if (found >= 0) dots.unshift(found)
      current = object
    } else if (current.type === 'ChainExpression') {
      current = current.expression as AnyNode
    } else {
      break
    }
  }
  return dots
}
