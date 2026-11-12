/**
 * The text `ariaSnapshot()` returns by default (`mode: 'ai'` or no mode): a short list of what an
 * agent can act on, in Playwright's own line syntax and with Playwright's own refs.
 *
 * Its input is the snapshot ops' `{ json, partial }`, the same tree snapshot.ts renders as
 * Playwright's text, so `page.getByRef('e5')` resolves a printed ref exactly as before. Every
 * element line is the vendored renderer's line for a copy of that one node without its children:
 * keys, attribute order, quoting and escaping stay Playwright's. This module only decides which
 * nodes print, which refs and markers they keep, and where the page's text goes:
 *
 *   kept block   dialog, alertdialog, alert, status   its line, then all of its text; a dialog's
 *                                                     ref, an alert's only when clickable
 *   section      main, navigation, form, iframe, ...  its line with its ref, content indented
 *   heading                                           its line, no ref
 *   actionable   a ref, and an interactive role or a pointer cursor     one line with its ref
 *   text         `- text:` lines, in the text form only
 *   other        no line; its content moves up to the level it sits in
 *
 * The text form is returned while it is at most TEXT_LIMIT_CHARS long, otherwise the interactive
 * form under a notice saying how much text was left out. Either is then capped at `maxTokens`;
 * a kept block after the cut is still printed. Rendering happens in the agent's realm, never in
 * a page, for the reason snapshot.ts gives.
 */
import {
  aboutTokens,
  CHARS_PER_TOKEN,
  cap,
  type Printed,
  type Section,
} from './snapshot-cap'
import type { AriaNodeJSON } from './vendored'
import { renderAriaSnapshotAsYaml } from './vendored'
import type { PartialFrame, SnapshotResult } from './wire'

/** Roles an agent acts on: the list desktop Neo's clickable observer uses. */
const INTERACTIVE = new Set<string>([
  'button',
  'link',
  'textbox',
  'checkbox',
  'radio',
  'combobox',
  'listbox',
  'menuitem',
  'menuitemcheckbox',
  'menuitemradio',
  'option',
  'searchbox',
  'slider',
  'spinbutton',
  'switch',
  'tab',
  'treeitem',
])

/** Regions an agent can scope a snapshot to (the cut line names one), so their refs print. */
const SECTION = new Set<string>([
  'banner',
  'navigation',
  'main',
  'complementary',
  'contentinfo',
  'search',
  'form',
  'region',
  'article',
  'iframe',
])

/**
 * Blocks whose text says what the controls around them mean (an error, a confirm question), so
 * their text prints even in the interactive form, and a cut never hides them.
 */
const KEPT = new Set<string>(['dialog', 'alertdialog', 'alert', 'status'])
const KEPT_WITH_REF = new Set<string>(['dialog', 'alertdialog'])

/**
 * Whether a kept block's line prints its ref: always for a dialog, which an agent scopes to; for
 * an alert or status only when it is actionable (a toast the user can click), with its marker.
 */
function keptRef(node: AriaNodeJSON): boolean {
  return KEPT_WITH_REF.has(node.role) || actionable(node)
}

/** 3,000 tokens: the longest text form returned when the script did not choose. */
export const TEXT_LIMIT_CHARS = 3_000 * CHARS_PER_TOKEN
export const DEFAULT_MAX_TOKENS = 8_000
/** A nameless control's own text is clipped to this in the interactive form. */
const OWN_TEXT_CLIP = 80
/** Past this many options, a native select lists only its selected ones without text. */
const LONG_SELECT = 12

export type SnapshotV1Options = {
  /** `true`: the text form; `false`: the interactive form; left out: chosen by length. */
  text?: boolean
  /** Prints each link's `/url`. */
  urls: boolean
  maxTokens: number
}

/** The v1 text for one snapshot op's `{ json, partial }`. */
export function renderSnapshotV1(
  result: SnapshotResult,
  options: SnapshotV1Options,
): string {
  const limit = options.maxTokens * CHARS_PER_TOKEN
  if (options.text === false)
    return cap(null, walk(result, options.urls, false), limit)
  const textForm = walk(result, options.urls, true)
  const textLength = joinedLength(textForm)
  if (options.text === true || textLength <= TEXT_LIMIT_CHARS)
    return cap(null, textForm, limit)
  const interactiveForm = walk(result, options.urls, false)
  const left = textLength - joinedLength(interactiveForm)
  const about = aboutTokens(left)
  const notice = `# text left out: about ${about} tokens. Pass { text: true } to include it.`
  return cap(notice, interactiveForm, limit)
}

/** One form: the tree walked with page text on or off, then partial frames with no iframe line. */
function walk(result: SnapshotResult, urls: boolean, text: boolean): Printed[] {
  const partials = new Map<string, PartialFrame[]>()
  for (const partial of result.partial)
    partials.set(partial.ref, [...(partials.get(partial.ref) ?? []), partial])
  const printer = new Printer(urls, partials)
  // A virtual root over the json array, so a locator's root node goes through the same rules:
  // a locator on `main` prints `- main [ref=e2]:` and its content.
  printer.walk(result.json, 0, text)
  for (const unplaced of partials.values())
    for (const partial of unplaced) printer.comment(partialComment(partial), 0)
  return printer.lines.map((line) => ({
    text:
      '  '.repeat(line.level) +
      (line.textLine
        ? renderAriaSnapshotAsYaml([{ role: 'text', text: line.body }])
        : line.body),
    ref: line.ref,
    section: line.section,
    keptEnd: line.keptEnd,
  }))
}

/** A printed line before indentation; a text line's body is its raw joined pieces. */
type Line = {
  level: number
  body: string
  textLine: boolean
  comment: boolean
  ref: boolean
  section?: Section
  keptEnd?: number
}

/**
 * One step of the walk. The steps wait on an explicit stack rather than the call stack: real
 * pages nest deeper than QuickJS's 512 KiB stack allows a recursive walk (an Amazon product
 * page overflowed it), and a block's closing step must run after all of its content.
 */
type Step =
  | { kind: 'node'; child: AriaNodeJSON | string; level: number; text: boolean }
  | { kind: 'piece'; raw: string; level: number; text: boolean }
  | { kind: 'kept'; node: AriaNodeJSON; header: number; level: number }
  | { kind: 'section'; node: AriaNodeJSON; header: number }

class Printer {
  readonly lines: Line[] = []
  /** The walk's pending steps; the next one is last. */
  private readonly steps: Step[] = []

  constructor(
    private readonly urls: boolean,
    /** The frames Rust could not read, by iframe ref; each is removed once printed. */
    private readonly partials: Map<string, PartialFrame[]>,
  ) {}

  /** Prints `children` at `level`, depth first in document order. */
  walk(
    children: readonly (AriaNodeJSON | string)[],
    level: number,
    text: boolean,
  ): void {
    this.content({ children: [...children] }, level, text)
    for (let step = this.steps.pop(); step; step = this.steps.pop()) {
      if (step.kind === 'node') this.node(step.child, step.level, step.text)
      else if (step.kind === 'piece')
        this.piece(step.raw, step.level, step.text)
      else if (step.kind === 'kept')
        this.closeKept(step.node, step.header, step.level)
      else this.closeSection(step.node, step.header)
    }
  }

  comment(body: string, level: number): void {
    this.lines.push({ level, body, textLine: false, comment: true, ref: false })
  }

  /** One child of the node being walked, by the first rule that matches it. */
  private node(
    child: AriaNodeJSON | string,
    level: number,
    text: boolean,
  ): void {
    if (typeof child === 'string') this.piece(child, level, text)
    else if (child.role === 'text') this.piece(child.text ?? '', level, text)
    else if (KEPT.has(child.role)) this.kept(child, level)
    else if (SECTION.has(child.role)) this.section(child, level, text)
    else if (child.role === 'heading') this.heading(child, level)
    else if (actionable(child)) this.actionable(child, level, text)
    else {
      if (child.role === 'img' && child.name && text)
        this.element(copy(child, { ref: false, urls: false }), level)
      // No line of its own: its text and children belong to the level it sits in.
      this.content(child, level, text)
    }
  }

  /**
   * Schedules a node's own `text` field, then its children, as the next steps. Children go on
   * in reverse so they come off in document order, ahead of the siblings already waiting.
   */
  private content(
    node: AriaNodeJSON | { children: (AriaNodeJSON | string)[] },
    level: number,
    text: boolean,
  ): void {
    const children = node.children ?? []
    for (let index = children.length - 1; index >= 0; index--)
      this.steps.push({ kind: 'node', child: children[index], level, text })
    if ('text' in node && node.text !== undefined)
      this.steps.push({ kind: 'piece', raw: node.text, level, text })
  }

  /**
   * A text piece joins the text line printed last, if that is at the same level, with one
   * space. Pieces stay raw until the form is done, so YAML quoting covers the whole line.
   */
  private piece(raw: string, level: number, text: boolean): void {
    if (!text || raw === '') return
    const last = this.lines[this.lines.length - 1]
    if (last?.textLine && last.level === level) last.body += ` ${raw}`
    else
      this.lines.push({
        level,
        body: raw,
        textLine: true,
        comment: false,
        ref: false,
      })
  }

  private kept(node: AriaNodeJSON, level: number): void {
    const header = this.header(node, level, keptRef(node))
    this.steps.push({ kind: 'kept', node, header, level })
    // Its text is what tells the agent what its buttons do, so it prints whatever the form.
    this.content(node, level + 1, true)
  }

  /** After a kept block's content: the leaf form, a colon, or no line at all. */
  private closeKept(node: AriaNodeJSON, header: number, level: number): void {
    const inside = this.lines.slice(header + 1)
    if (inside.length === 1 && inside[0].textLine) {
      // All of its content is one piece of text: the leaf form, `- alert: Invalid password.`
      this.lines.length = header
      this.element(
        copy(node, { ref: keptRef(node), urls: false, text: inside[0].body }),
        level,
      )
    } else if (!this.close(header, Boolean(node.name))) return
    this.lines[header].keptEnd = this.lines.length
  }

  private section(node: AriaNodeJSON, level: number, text: boolean): void {
    const header = this.header(node, level, true)
    if (node.role === 'iframe' && node.ref !== undefined) {
      // Under its iframe line, as snapshot.ts prints it for Playwright's text.
      for (const partial of this.partials.get(node.ref) ?? [])
        this.comment(partialComment(partial), level + 1)
      this.partials.delete(node.ref)
    }
    this.steps.push({ kind: 'section', node, header })
    this.content(node, level + 1, text)
  }

  /** After a section's content: its colon, or no line when nothing printed inside it. */
  private closeSection(node: AriaNodeJSON, header: number): void {
    // An iframe keeps its line with nothing inside: its `# partial` comment hangs under it.
    if (
      this.close(header, node.role === 'iframe') &&
      this.lines[header].body.endsWith(':') &&
      node.ref !== undefined
    )
      this.lines[header].section = {
        ref: node.ref,
        label: node.name
          ? `${node.role} ${JSON.stringify(node.name)}`
          : node.role,
        end: this.lines.length,
      }
  }

  private heading(node: AriaNodeJSON, level: number): void {
    const own = node.name ? '' : ownText(node)
    this.element(
      copy(node, {
        ref: actionable(node),
        urls: false,
        text: own === '' ? undefined : own,
      }),
      level,
    )
    // A link inside the heading gets its own line; the heading's text is not repeated.
    this.content(node, level, false)
  }

  private actionable(node: AriaNodeJSON, level: number, text: boolean): void {
    let value: string | undefined
    // A text field that differs from the name: how a textbox shows its value.
    if (node.text !== undefined && node.text !== node.name) value = node.text
    else if (!node.name) {
      const own = ownText(node)
      if (own !== '') value = text ? own : clip(own, OWN_TEXT_CLIP)
    }
    const line = copy(node, { ref: true, urls: this.urls, text: value })
    const choices =
      node.role === 'combobox' || node.role === 'listbox'
        ? nativeOptions(node)
        : []
    if (choices.length === 0) this.element(line, level)
    else {
      // A native select: its options carry no refs; they are its value and its choices.
      this.header(node, level, true, line)
      this.lines[this.lines.length - 1].body += ':'
      const shown =
        text || choices.length <= LONG_SELECT
          ? choices
          : choices.filter((option) => option.selected === true)
      for (const option of shown)
        this.element(
          copy(option, { ref: false, urls: false, text: option.text }),
          level + 1,
        )
      if (shown.length < choices.length)
        this.lines.push({
          level: level + 1,
          body: `- text: ${choices.length - shown.length} more options`,
          textLine: false,
          comment: false,
          ref: false,
        })
    }
    // Nested controls get lines of their own; the control's own text is not printed again.
    this.content(node, level, false)
  }

  /** The vendored renderer's lines for one childless node copy, at `level`. */
  private element(line: AriaNodeJSON, level: number): void {
    const bodies = renderAriaSnapshotAsYaml([line]).split('\n')
    for (const [index, body] of bodies.entries())
      this.lines.push({
        level,
        body,
        textLine: false,
        comment: false,
        ref: index === 0 && line.ref !== undefined,
      })
  }

  /** A block's first line, its colon added by `close`; returns its index. */
  private header(
    node: AriaNodeJSON,
    level: number,
    ref: boolean,
    line: AriaNodeJSON = copy(node, { ref, urls: false }),
  ): number {
    // Only the key: a block's text and props are its content, not its line.
    const { text: _text, url: _url, placeholder: _placeholder, ...key } = line
    this.lines.push({
      level,
      body: renderAriaSnapshotAsYaml([key]),
      textLine: false,
      comment: false,
      ref: key.ref !== undefined,
    })
    return this.lines.length - 1
  }

  /**
   * Ends the block opened at `header`: a colon when an element or text line printed inside it.
   * With nothing printed inside, its line stays only when `keepEmpty`. Says whether it stayed.
   */
  private close(header: number, keepEmpty: boolean): boolean {
    const inside = this.lines.slice(header + 1)
    if (inside.some((line) => !line.comment)) {
      this.lines[header].body += ':'
      return true
    }
    if (keepEmpty || inside.length > 0) return true
    this.lines.pop()
    return false
  }
}

/** Has a ref, and an interactive role or a pointer cursor: something the agent can act on. */
function actionable(node: AriaNodeJSON): boolean {
  return (
    node.ref !== undefined &&
    (INTERACTIVE.has(node.role) || node.cursor === 'pointer')
  )
}

/**
 * Playwright's node for one line: no children, the ref only where the agent can act on it or
 * scope to it, `[cursor=pointer]` only where it is what says a non-control is clickable, and
 * `/url` only when the script asked for urls. `/placeholder` stays, as Playwright prints it.
 */
function copy(
  node: AriaNodeJSON,
  keep: { ref: boolean; urls: boolean; text?: string },
): AriaNodeJSON {
  const { children: _children, ref, cursor, url, text: _text, ...line } = node
  const copied: AriaNodeJSON = { ...line }
  if (keep.ref && ref !== undefined) {
    copied.ref = ref
    if (cursor !== undefined && !INTERACTIVE.has(node.role))
      copied.cursor = cursor
  }
  if (keep.urls && url !== undefined) copied.url = url
  if (keep.text !== undefined) copied.text = keep.text
  return copied
}

/**
 * A native select's options, which carry no refs, in document order: `<optgroup>`s are `group`
 * nodes, which print no line, so their options are collected through them.
 */
function nativeOptions(select: AriaNodeJSON): AriaNodeJSON[] {
  const options: AriaNodeJSON[] = []
  const visit = (children: AriaNodeJSON['children']): void => {
    for (const child of children ?? []) {
      if (typeof child === 'string') continue
      if (child.role === 'option' && !child.ref) options.push(child)
      else if (child.role === 'group' && !actionable(child))
        visit(child.children)
    }
  }
  visit(select.children)
  return options
}

/** Every text piece under `node` that is not inside a nested actionable node, space-joined. */
function ownText(node: AriaNodeJSON): string {
  const pieces: string[] = []
  // Depth first in document order on an explicit stack, as the walk is, for deep trees.
  const stack: (AriaNodeJSON | string)[] = [node]
  for (let item = stack.pop(); item !== undefined; item = stack.pop()) {
    if (typeof item === 'string') {
      pieces.push(item)
      continue
    }
    if (item.text !== undefined) pieces.push(item.text)
    const children = item.children ?? []
    for (let index = children.length - 1; index >= 0; index--) {
      const child = children[index]
      if (
        typeof child === 'string' ||
        child.role === 'text' ||
        !actionable(child)
      )
        stack.push(child)
    }
  }
  return pieces.filter((piece) => piece !== '').join(' ')
}

function clip(text: string, length: number): string {
  return text.length <= length ? text : `${text.slice(0, length).trimEnd()}…`
}

function partialComment(partial: PartialFrame): string {
  return `# partial: ${partial.frame} not read (${partial.reason})`
}

function joinedLength(lines: readonly Printed[]): number {
  return lines.reduce(
    (length, line, index) => length + line.text.length + (index > 0 ? 1 : 0),
    0,
  )
}
