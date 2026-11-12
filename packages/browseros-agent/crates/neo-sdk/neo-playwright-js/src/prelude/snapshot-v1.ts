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
 *   kept block   dialog, alertdialog, alert, status   its line, then all of its text
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

/** A token is counted as four characters of JS string length, here and in the notice. */
const CHARS_PER_TOKEN = 4
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
  const left = (textLength - joinedLength(interactiveForm)) / CHARS_PER_TOKEN
  const about = Math.max(100, Math.round(left / 100) * 100)
  const notice = `# text left out: about ${about} tokens. Pass { text: true } to include it.`
  return cap(notice, interactiveForm, limit)
}

/** One output line, indented, with what the cap needs to know about it. */
type Printed = {
  text: string
  /** It prints a `[ref=…]`. */
  ref: boolean
  /** A section line with content: the ref the cut line can suggest. */
  section?: string
  /** The first line of a kept block: the index after its last line. */
  keptEnd?: number
}

/** One form: the tree walked with page text on or off, then partial frames with no iframe line. */
function walk(result: SnapshotResult, urls: boolean, text: boolean): Printed[] {
  const partials = new Map<string, PartialFrame[]>()
  for (const partial of result.partial)
    partials.set(partial.ref, [...(partials.get(partial.ref) ?? []), partial])
  const printer = new Printer(urls, partials)
  // A virtual root over the json array, so a locator's root node goes through the same rules:
  // a locator on `main` prints `- main [ref=e2]:` and its content.
  printer.children(result.json, 0, text)
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
  section?: string
  keptEnd?: number
}

class Printer {
  readonly lines: Line[] = []

  constructor(
    private readonly urls: boolean,
    /** The frames Rust could not read, by iframe ref; each is removed once printed. */
    private readonly partials: Map<string, PartialFrame[]>,
  ) {}

  children(
    children: readonly (AriaNodeJSON | string)[] | undefined,
    level: number,
    text: boolean,
  ): void {
    for (const child of children ?? []) this.node(child, level, text)
  }

  /** One child of the node being walked, by the first rule that matches it. */
  node(child: AriaNodeJSON | string, level: number, text: boolean): void {
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

  comment(body: string, level: number): void {
    this.lines.push({ level, body, textLine: false, comment: true, ref: false })
  }

  /** A node's own `text` field, then its children. */
  private content(node: AriaNodeJSON, level: number, text: boolean): void {
    if (node.text !== undefined) this.piece(node.text, level, text)
    this.children(node.children, level, text)
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
    const header = this.header(node, level, KEPT_WITH_REF.has(node.role))
    // Its text is what tells the agent what its buttons do, so it prints whatever the form.
    this.content(node, level + 1, true)
    const inside = this.lines.slice(header + 1)
    if (inside.length === 1 && inside[0].textLine) {
      // All of its content is one piece of text: the leaf form, `- alert: Invalid password.`
      this.lines.length = header
      this.element(
        copy(node, { ref: false, urls: false, text: inside[0].body }),
        level,
      )
    } else if (!this.close(header, Boolean(node.name))) return
    this.lines[header].keptEnd = this.lines.length
  }

  private section(node: AriaNodeJSON, level: number, text: boolean): void {
    const header = this.header(node, level, true)
    const iframe = node.role === 'iframe'
    if (iframe && node.ref !== undefined) {
      // Under its iframe line, as snapshot.ts prints it for Playwright's text.
      for (const partial of this.partials.get(node.ref) ?? [])
        this.comment(partialComment(partial), level + 1)
      this.partials.delete(node.ref)
    }
    this.content(node, level + 1, text)
    // An iframe keeps its line with nothing inside: its `# partial` comment hangs under it.
    if (this.close(header, iframe) && this.lines[header].body.endsWith(':'))
      this.lines[header].section = node.ref
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
    this.children(node.children, level, false)
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
        ? (node.children ?? []).filter(
            (child): child is AriaNodeJSON =>
              typeof child !== 'string' &&
              child.role === 'option' &&
              !child.ref,
          )
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
    this.children(node.children, level, false)
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

/** Every text piece under `node` that is not inside a nested actionable node, space-joined. */
function ownText(node: AriaNodeJSON): string {
  const pieces: string[] = []
  const visit = (parent: AriaNodeJSON): void => {
    if (parent.text !== undefined) pieces.push(parent.text)
    for (const child of parent.children ?? []) {
      if (typeof child === 'string') pieces.push(child)
      else if (child.role === 'text') pieces.push(child.text ?? '')
      else if (!actionable(child)) visit(child)
    }
  }
  visit(node)
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

/**
 * `notice` and `lines` within `limit` characters: whole lines from the top while they fit (the
 * notice counts), then one `# cut:` line, then every kept block after the cut, whole and outside
 * the limit. Sites put dialogs at the end of the document; a cut must never hide one.
 */
function cap(
  notice: string | null,
  lines: readonly Printed[],
  limit: number,
): string {
  const head = notice === null ? [] : [notice]
  const sequence = [...head, ...lines.map((line) => line.text)]
  const whole = sequence.join('\n')
  if (whole.length <= limit) return whole
  // `used` counts the newline before each line but the first.
  let used = -1
  let taken = 0
  while (
    taken < sequence.length &&
    used + 1 + sequence[taken].length <= limit
  ) {
    used += 1 + sequence[taken].length
    taken += 1
  }
  const top = head.slice(0, taken)
  let cut = Math.max(0, taken - head.length)
  // A kept block the cut would split starts after the cut instead, so it prints whole.
  for (let index = 0; index < cut; index++) {
    const end = lines[index].keptEnd
    if (end !== undefined && end > cut) {
      cut = index
      break
    }
  }
  const blocks: string[] = []
  let more = 0
  let withRefs = 0
  let section: string | undefined
  for (let index = cut; index < lines.length; ) {
    const line = lines[index]
    if (line.keptEnd !== undefined) {
      // Moved out of its section, so it starts at the left edge with its inner indentation.
      const indent = line.text.length - line.text.trimStart().length
      for (const kept of lines.slice(index, line.keptEnd))
        blocks.push(kept.text.slice(indent))
      index = line.keptEnd
      continue
    }
    more += 1
    if (line.ref) withRefs += 1
    section ??= line.section
    index += 1
  }
  const shown = [...top, ...lines.slice(0, cut).map((line) => line.text)]
  if (more > 0) {
    // The suggested section is the first one the cut left out; with none, only the cap is named.
    const next =
      section === undefined
        ? 'Pass a larger maxTokens.'
        : `Snapshot one section, e.g. page.getByRef('${section}').ariaSnapshot(), or pass a larger maxTokens.`
    shown.push(`# cut: ${more} more lines (${withRefs} with refs). ${next}`)
  }
  return [...shown, ...blocks].join('\n')
}
