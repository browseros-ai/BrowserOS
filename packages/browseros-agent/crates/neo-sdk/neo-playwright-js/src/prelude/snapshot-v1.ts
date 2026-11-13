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
 *   field        a value or checked state, even without a ref          one line with its state
 *   text         `- text:` lines, in the text form only; none that holds only separators
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
  partialHead,
  type Section,
} from './snapshot-cap'
import type { AriaNodeJSON } from './vendored'
import { renderAriaSnapshotAsYaml } from './vendored'
import type {
  PartialFrame,
  SnapshotForm,
  SnapshotResult,
  TextForm,
} from './wire'

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

/** Fields keep their state visible even when the engine gives them no pointer target. */
const FIELD = new Set<string>([
  'textbox',
  'searchbox',
  'spinbutton',
  'slider',
  'combobox',
  'listbox',
  'checkbox',
  'radio',
  'switch',
  'menuitemcheckbox',
  'menuitemradio',
])

/**
 * Regions an agent can scope a snapshot to (the cut line names one), so their refs print. The
 * cap of `mode: 'full'` (snapshot.ts) offers the same regions, so both formats name one thing.
 */
export const SECTION = new Set<string>([
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
 * their text prints even in the interactive form, and a cut never hides them, in either format.
 */
export const KEPT = new Set<string>([
  'dialog',
  'alertdialog',
  'alert',
  'status',
])
const KEPT_WITH_REF = new Set<string>(['dialog', 'alertdialog'])

/**
 * A node's own `text` as Neo's formats print it: the v1 text, and `mode: 'full'` (snapshot.ts).
 * An iframe's text is the fallback markup written between its tags (`<iframe …><p>Your browser
 * does not support iframes.</p></iframe>`), which the HTML parser keeps as raw text and no
 * browser shows. The engine reads it as the iframe node's text and Rust stitches the frame's
 * content under the same node, as children. Playwright's renderer prints a node that has text
 * as one line and never visits its children, so printing that text would lose the frame (a
 * payment page's card fields). Playwright's own YAML (`mode: 'default'` and the conformance
 * switch) keeps it, as upstream prints it.
 */
export function shownText(node: AriaNodeJSON): string | undefined {
  return node.role === 'iframe' ? undefined : node.text
}

/** How the cut line names a section: its role, and its quoted name when it has one. */
export function sectionLabel(node: AriaNodeJSON): string {
  return node.name ? `${node.role} ${JSON.stringify(node.name)}` : node.role
}

/**
 * Whether a kept block's line prints its ref: always for a dialog, which an agent scopes to; for
 * an alert or status only when it is actionable (a toast the user can click), with its marker.
 */
function keptRef(node: AriaNodeJSON): boolean {
  return KEPT_WITH_REF.has(node.role) || actionable(node)
}

/** 3,000 tokens: the longest text form returned when the script did not choose. */
const TEXT_LIMIT_CHARS = 3_000 * CHARS_PER_TOKEN
export const DEFAULT_MAX_TOKENS = 8_000
/** A nameless control's own text, or a link's url label, is clipped to this without text. */
const OWN_TEXT_CLIP = 80
/** Past this many options, a native select lists only its selected ones without text. */
const LONG_SELECT = 12
/**
 * A joined text line made only of these says nothing: Hacker News printed `- text: "|"` between
 * its links, 12% of its front page. Currency signs, `%`, `@`, `#`, `*`, `+`, `=`, `?`, `!` and `&`
 * are not here: beside a field they mean something.
 */
const SEPARATORS_ONLY = /^[\s|·•‧∙/\\\-–—‐›»«‹><:;,.()[\]{}]*$/

export type SnapshotV1Options = {
  /** `true`: the text form; `false`: the interactive form; left out: chosen by length. */
  text?: boolean
  /** Prints each link's `/url`. */
  urls: boolean
  maxTokens: number
}

/**
 * The v1 options as the script passed them, each `undefined` when left out. A delta takes the
 * ones left out from its baseline's form, so "left out" has to survive until then.
 */
export type PassedV1Options = {
  text?: boolean
  urls?: boolean
  maxTokens?: number
}

/** `passed` with Neo's defaults for what the script left out; `text` stays to the length rule. */
export function withDefaults(passed: PassedV1Options): SnapshotV1Options {
  return {
    text: passed.text,
    urls: passed.urls ?? false,
    maxTokens: passed.maxTokens ?? DEFAULT_MAX_TOKENS,
  }
}

/**
 * One v1 text and how it was rendered. They travel together because a delta baseline is stored
 * with the form of the very text the caller was given (snapshot-delta.ts).
 */
export type RenderedV1 = { text: string; form: SnapshotForm }

/** The v1 text for one snapshot op's `{ json, partial }`, and how it was rendered. */
export function renderSnapshotV1(
  result: SnapshotResult,
  options: SnapshotV1Options,
): RenderedV1 {
  const { urls, maxTokens } = options
  const textForm = options.text === false ? null : walk(result, urls, true)
  let text: TextForm
  if (textForm === null) text = 'never'
  else if (options.text === true) text = 'always'
  else text = joinedLength(textForm) <= TEXT_LIMIT_CHARS ? 'kept' : 'leftOut'
  const form = { text, urls, maxTokens }
  return { text: renderForm(result, form, textForm), form }
}

/**
 * The v1 text rendered exactly as `form` says, the length rule not asked again: `kept` and
 * `always` with the page's text whatever its length, `leftOut` under the notice whatever its
 * length, `never` without either. A delta compares a new snapshot rendered so with its
 * baseline, so the two differ only where the page does. Not an option a script can pass.
 */
export function renderSnapshotV1As(
  result: SnapshotResult,
  form: SnapshotForm,
): string {
  return renderForm(result, form, null)
}

/** `form`'s text, reusing the text-form walk when the caller already made it. */
function renderForm(
  result: SnapshotResult,
  form: SnapshotForm,
  walked: Printed[] | null,
): string {
  const limit = form.maxTokens * CHARS_PER_TOKEN
  const partial = partialHead(result)
  if (form.text === 'never')
    return cap([], partial, walk(result, form.urls, false), limit)
  const textForm = walked ?? walk(result, form.urls, true)
  if (form.text !== 'leftOut') return cap([], partial, textForm, limit)
  const interactiveForm = walk(result, form.urls, false)
  const left = joinedLength(textForm) - joinedLength(interactiveForm)
  const about = aboutTokens(left)
  const notice = `# text left out: about ${about} tokens. Pass { text: true } to include it.`
  return cap([notice], partial, interactiveForm, limit)
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
  return printer.finish().map((line) => ({
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

  /**
   * The lines as printed, once the walk is done. A separator-only text line is judged only now,
   * on its joined text: a price arrives as `$`, `7`, `.`, `49`, and while its line could still
   * grow, `$` alone is not a reason to drop it. Block ends are indices into these lines, so each
   * moves back by the lines dropped before it.
   */
  finish(): Line[] {
    const printed: Line[] = []
    const moved: number[] = []
    for (const line of this.lines) {
      moved.push(printed.length)
      if (!separatorsOnly(line)) printed.push(line)
    }
    moved.push(printed.length)
    for (const line of printed) {
      if (line.section) line.section.end = moved[line.section.end]
      if (line.keptEnd !== undefined) line.keptEnd = moved[line.keptEnd]
    }
    return printed
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
    else if (actionable(child) || fieldWithoutRef(child))
      this.control(child, level, text)
    else {
      if (child.role === 'img') {
        if (child.name && text)
          this.element(copy(child, { ref: false, urls: false }), level)
      } else {
        const named = nameAsContent(child)
        if (named !== undefined) this.piece(named, level, text)
      }
      // No line of its own: its text and children belong to the level it sits in.
      this.content(child, level, text)
    }
  }

  /**
   * Schedules a node's own `text` field (none for an iframe: `shownText`), then its children,
   * as the next steps. Children go on in reverse so they come off in document order, ahead of
   * the siblings already waiting.
   */
  private content(
    node: AriaNodeJSON | { children: (AriaNodeJSON | string)[] },
    level: number,
    text: boolean,
  ): void {
    const children = node.children ?? []
    for (let index = children.length - 1; index >= 0; index--)
      this.steps.push({ kind: 'node', child: children[index], level, text })
    const own = 'role' in node ? shownText(node) : undefined
    if (own !== undefined)
      this.steps.push({ kind: 'piece', raw: own, level, text })
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
    if (
      inside.length === 1 &&
      inside[0].textLine &&
      !separatorsOnly(inside[0])
    ) {
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
        label: sectionLabel(node),
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

  private control(node: AriaNodeJSON, level: number, text: boolean): void {
    let value: string | undefined
    // A text field that differs from the name: how a textbox shows its value.
    if (node.text !== undefined && node.text !== node.name) value = node.text
    else if (fieldWithoutRef(node) && node.text === undefined) {
      // Contenteditable values can be paragraph children instead of a `text` field. Keep
      // that value on the field's line before the child walk suppresses repeated text.
      const own = ownText(node)
      if (own !== '') value = own
    } else if (!node.name) {
      // A link with no words of its own still says where it goes, unless its `/url` line does.
      const label =
        ownText(node) ||
        (node.role === 'link' && !this.urls ? (node.url ?? '') : '')
      if (label !== '') value = text ? label : clip(label, OWN_TEXT_CLIP)
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
   * A text line of separators alone is final here (nothing after the block joins a line inside
   * it) and does not count. With nothing printed inside, its line stays only when `keepEmpty`
   * or a comment hangs under it; otherwise it goes with its separator lines, so the text before
   * and after it can join as if it was never there. Says whether it stayed.
   */
  private close(header: number, keepEmpty: boolean): boolean {
    const inside = this.lines.slice(header + 1)
    if (inside.some((line) => !line.comment && !separatorsOnly(line))) {
      this.lines[header].body += ':'
      return true
    }
    if (keepEmpty || inside.some((line) => line.comment)) return true
    this.lines.length = header
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

function fieldWithoutRef(node: AriaNodeJSON): boolean {
  return node.ref === undefined && FIELD.has(node.role)
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

/** A text line whose joined text holds only separators: it is never printed. */
function separatorsOnly(line: Line): boolean {
  return line.textLine && SEPARATORS_ONLY.test(line.body)
}

/**
 * The name of a node under which nothing prints text, which is then its content. The engine
 * names a table cell, a list item or a group from its text and drops the text child that
 * repeats the name: `<td>$7.49</td>` is `cell "$7.49"` with no children, so without this the
 * price would print nowhere, even with text on. An image's alt text arrives the same way, and is
 * the only label a link around a badge has. Children that say nothing do not change that: a
 * grid's `columnheader "Full name"` around an empty clickable `div`, or a figure named by its
 * caption around a link with no name, has its words only in its name. A node with a child that
 * prints text keeps that child's lines instead (a cell holding a link), so its name, derived from
 * them, is never printed twice. A node with a `text` field has that text as its content instead.
 * A heading's name is never content: the heading prints its own line (rule 4), just below the
 * control whose own text would repeat it. In the walk an image prints its own line (rule 6)
 * before this is asked.
 */
function nameAsContent(node: AriaNodeJSON): string | undefined {
  if (
    !node.name ||
    node.text !== undefined ||
    node.role === 'heading' ||
    fieldWithoutRef(node)
  )
    return undefined
  return saysNothing(node.children) ? node.name : undefined
}

/**
 * Whether nothing under `children` prints text: no string, and no node with a printed `text`
 * field (`shownText`) or a name, at any depth (an empty one prints nothing, as in the walk).
 * Stops at the first one, and walks an explicit stack, as the walk does, for trees deeper than
 * the call stack.
 */
function saysNothing(children: AriaNodeJSON['children']): boolean {
  const pending: (AriaNodeJSON | string)[] = []
  const add = (nodes: AriaNodeJSON['children']) => {
    for (const node of nodes ?? []) pending.push(node)
  }
  add(children)
  for (let item = pending.pop(); item !== undefined; item = pending.pop()) {
    if (typeof item === 'string') {
      if (item !== '') return false
    } else if (shownText(item) || item.name) return false
    else add(item.children)
  }
  return true
}

/**
 * Every text piece under `node` outside a nested actionable node or ref-less field, space-joined.
 * Those controls print their own lines, so their names and values cannot label their drawing
 * too. Other content-only names count as pieces (an image's alt text, a cell's text).
 * `shownText` skips an iframe's fallback markup while keeping its frame's text.
 */
function ownText(node: AriaNodeJSON): string {
  const pieces: string[] = []
  // Depth first in document order on an explicit stack, as the walk is, for deep trees.
  const stack: (AriaNodeJSON | string)[] = [node]
  for (let item = stack.pop(); item !== undefined; item = stack.pop()) {
    if (typeof item === 'string') {
      pieces.push(item)
      continue
    }
    const shown = shownText(item)
    if (shown !== undefined) pieces.push(shown)
    const children = item.children ?? []
    for (let index = children.length - 1; index >= 0; index--) {
      const child = children[index]
      if (typeof child === 'string' || child.role === 'text') stack.push(child)
      else if (!actionable(child) && !fieldWithoutRef(child))
        stack.push(nameAsContent(child) ?? child)
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
