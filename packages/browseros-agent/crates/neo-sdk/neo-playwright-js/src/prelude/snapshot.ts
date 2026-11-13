/**
 * Renders the snapshot ops' result as Playwright's YAML: the text `ariaSnapshot()` returns with
 * `mode: 'default'`, with `mode: 'full'` (the engine's `'ai'` tree, capped), and with
 * `mode: 'ai'` under the conformance switch below. The default text format is snapshot-v1.ts,
 * made from the same result.
 *
 * Rust stitches every frame's JSON and applies Neo's naming rule; the text is made here, in the agent runtime, as upstream makes it in its server and
 * never in a page. Rendering in the root frame's world would copy cross-site frames' text into
 * the embedding site's renderer process. The YAML is the vendored renderer's, byte for byte;
 * Neo adds only one comment per frame it could not read, under that frame's iframe line:
 *
 *   - iframe [ref=e7]
 *     # partial: https://pay.example/form not read (timeout after 2000 ms)
 *
 * `mode: 'full'` differs from Playwright's text in one thing: an iframe's fallback markup does
 * not print, so its frame's content does (snapshot-v1.ts `shownText`).
 */
import { CHARS_PER_TOKEN, cap, type Printed, partialHead } from './snapshot-cap'
import { KEPT, SECTION, sectionLabel, shownText } from './snapshot-v1'
import {
  type AriaNodeJSON,
  type AriaSnapshotJSON,
  renderAriaSnapshotAsYaml,
} from './vendored'
import type { PartialFrame, SnapshotResult } from './wire'

/**
 * The conformance switch. Playwright's own spec tests pin its exact snapshot text, so the spec
 * harness (neo-playwright-tests/node/claw-loader.mjs) sets `__neo_upstream_snapshots` on the
 * prelude's globalThis: then `ariaSnapshot()` with no mode means `'default'` and `mode: 'ai'`
 * prints Playwright's text, both as they did before the v1 format. Production hosts never set
 * it. Read once, as the prelude starts and before the agent's function exists, so a script that
 * sets the global changes nothing.
 */
export const upstreamSnapshots: boolean =
  Reflect.get(globalThis, '__neo_upstream_snapshots') === true

/** The offer a cut full-mode text makes: a section in the same mode, not the v1 list. */
const FULL_CALL = "ariaSnapshot({ mode: 'full' })"

/** One line of the YAML, and the node it is the line of (none for text, props and comments). */
type YamlLine = { text: string; node?: AriaNodeJSON }

function comment(partial: PartialFrame): string {
  return `# partial: ${partial.frame} not read (${partial.reason})`
}

/**
 * The vendored renderer's YAML for `json`, line for line, each element's line with its node.
 *
 * The renderer recurses once per tree level, and an `'ai'` tree keeps every wrapper, so a page
 * stitched from nested frames could nest deeper than QuickJS's 512 KiB stack allows. Here the
 * walk keeps its pending children on an explicit stack, and each node's own lines (its key, its
 * text or its `/url` and `/placeholder`) are the renderer's for a copy of that node without
 * children, so keys, quoting and escaping stay Playwright's byte for byte. Alone, only a node
 * with children and neither text nor props renders differently: as a leaf, so its line gains the
 * colon that opens its block. Like the renderer, a node with text prints no children.
 *
 * `keepFallback` false is `mode: 'full'`: an iframe's own text, fallback markup no browser
 * shows, is dropped (`shownText`), so the iframe opens a block of its frame's content as an
 * iframe without fallback does. True is Playwright's text, the fallback line and no frame.
 */
function treeLines(json: AriaSnapshotJSON, keepFallback: boolean): YamlLine[] {
  const lines: YamlLine[] = []
  const steps: { child: AriaNodeJSON | string; depth: number }[] = []
  const later = (
    children: readonly (AriaNodeJSON | string)[],
    depth: number,
  ) => {
    for (let index = children.length - 1; index >= 0; index--)
      steps.push({ child: children[index], depth })
  }
  later(json, 0)
  for (let step = steps.pop(); step; step = steps.pop()) {
    const indent = '  '.repeat(step.depth)
    const { child } = step
    if (typeof child === 'string' || child.role === 'text') {
      const text = typeof child === 'string' ? child : child.text || ''
      const line = renderAriaSnapshotAsYaml([{ role: 'text', text }])
      if (line !== '') lines.push({ text: indent + line })
      continue
    }
    const { children, text: _text, ...own } = child
    const text = keepFallback ? child.text : shownText(child)
    const alone: AriaNodeJSON = text === undefined ? own : { ...own, text }
    const [key, ...rest] = renderAriaSnapshotAsYaml([alone]).split('\n')
    const block =
      text === undefined &&
      own.url === undefined &&
      own.placeholder === undefined &&
      (children?.length ?? 0) > 0
    lines.push({ text: indent + key + (block ? ':' : ''), node: child })
    for (const line of rest) lines.push({ text: indent + line })
    if (text === undefined && children) later(children, step.depth + 1)
  }
  return lines
}

/** The iframe line for `ref`, by its node, or by a scan when its node was not placed. */
function iframeLine(lines: readonly YamlLine[], ref: string): number {
  const placed = lines.findIndex((line) => line.node?.ref === ref)
  if (placed !== -1) return placed
  const tag = `[ref=${ref}]`
  return lines.findIndex(
    (line) => /^\s*- /.test(line.text) && line.text.includes(tag),
  )
}

/**
 * The YAML's lines for one snapshot op's `{ json, partial }`, each element's with its node;
 * `keepFallback` as `treeLines` takes it.
 */
function yamlLines(result: SnapshotResult, keepFallback: boolean): YamlLine[] {
  const lines = treeLines(result.json, keepFallback)
  if (result.partial.length === 0) return lines
  const placed = result.partial.map((partial) => ({
    partial,
    line: iframeLine(lines, partial.ref),
  }))
  const unplaced: YamlLine[] = []
  // From the bottom up, so each insertion leaves the lines above it where they were.
  for (const { partial, line } of placed.sort((a, b) => b.line - a.line)) {
    if (line === -1) {
      unplaced.unshift({ text: comment(partial) })
      continue
    }
    const indent = /^\s*/.exec(lines[line]?.text ?? '')?.[0] ?? ''
    lines.splice(line + 1, 0, { text: `${indent}  ${comment(partial)}` })
  }
  return [...lines, ...unplaced]
}

/**
 * Playwright's YAML for one snapshot op's `{ json, partial }`: `mode: 'default'`, and every mode
 * under the conformance switch.
 */
export function renderSnapshot(result: SnapshotResult): string {
  return yamlLines(result, true)
    .map((line) => line.text)
    .join('\n')
}

/**
 * `mode: 'full'`: the YAML of the engine's `'ai'` tree, every element and all the text, within
 * `maxTokens` by the v1 format's cap (snapshot-cap.ts). It fits on most single sections and on
 * few whole pages (9k tokens at the median of 31 live pages, 52k at most), so the cap is what
 * keeps one careless call from costing what the v1 format saves. When it fits, the text is
 * `renderSnapshot`'s byte for byte, except under an iframe with fallback markup between its
 * tags: there it prints the frame's content where Playwright prints only that markup, so it
 * never lacks a ref the v1 text prints.
 */
export function renderFullSnapshot(
  result: SnapshotResult,
  maxTokens: number,
): string {
  return cap(
    [],
    partialHead(result),
    printed(yamlLines(result, false)),
    maxTokens * CHARS_PER_TOKEN,
    FULL_CALL,
  )
}

/**
 * What the cap needs of each YAML line. A node's block is its line and every deeper-indented
 * line after it (its props, text, children and `# partial` comments), found with an explicit
 * stack: a real page nests deeper than QuickJS's stack allows a recursive walk. A kept block
 * (dialog, alert, ...) is never split and is printed past the cut; a section with a ref and
 * something under it is what the cut line offers, as in the v1 format.
 */
function printed(lines: readonly YamlLine[]): Printed[] {
  const ends = lines.map(() => lines.length)
  const open: { index: number; indent: number }[] = []
  for (let index = 0; index < lines.length; index++) {
    const text = lines[index].text
    const indent = text.length - text.trimStart().length
    for (
      let last = open.at(-1);
      last !== undefined && last.indent >= indent;
      last = open.at(-1)
    ) {
      ends[last.index] = index
      open.pop()
    }
    open.push({ index, indent })
  }
  return lines.map(({ text, node }, index): Printed => {
    // As the rule words it, a line that prints `[ref=`: page text that says so counts too.
    const line: Printed = { text, ref: text.includes('[ref=') }
    const end = ends[index]
    if (node === undefined) return line
    if (KEPT.has(node.role)) line.keptEnd = end
    else if (
      SECTION.has(node.role) &&
      node.ref !== undefined &&
      end > index + 1
    )
      line.section = { ref: node.ref, label: sectionLabel(node), end }
    return line
  })
}
