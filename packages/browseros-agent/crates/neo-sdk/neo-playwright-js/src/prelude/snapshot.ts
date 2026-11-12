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
 */
import { CHARS_PER_TOKEN, cap, type Printed } from './snapshot-cap'
import { KEPT, SECTION, sectionLabel } from './snapshot-v1'
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
 * Whether `text` is `node`'s own line. `lineToNode` indexes the renderer's lines, which a key
 * with a raw newline would shift once split; a line that does not start with the node's key
 * gets no node, so a shift can never make the cap treat a text line as a section or a dialog.
 */
function isLineOf(text: string, node: AriaNodeJSON): boolean {
  const line = text.trimStart()
  const start = line.startsWith("- '") ? 3 : 2
  const after = line.charAt(start + node.role.length)
  if (
    !line.startsWith('- ') ||
    line.slice(start, start + node.role.length) !== node.role ||
    !(after === '' || after === ' ' || after === ':' || after === "'")
  )
    return false
  return node.ref === undefined || line.includes(`[ref=${node.ref}]`)
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

/** The YAML's lines for one snapshot op's `{ json, partial }`, each element's with its node. */
function yamlLines(result: SnapshotResult): YamlLine[] {
  const json: AriaSnapshotJSON = result.json
  const lineToNode = new Map<number, AriaNodeJSON>()
  const yaml = renderAriaSnapshotAsYaml(json, { lineToNode })
  const lines: YamlLine[] =
    yaml === '' ? [] : yaml.split('\n').map((text) => ({ text }))
  for (const [index, node] of lineToNode) {
    const line = lines[index]
    if (line !== undefined && isLineOf(line.text, node)) line.node = node
  }
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

/** The YAML for one snapshot op's `{ json, partial }`. */
export function renderSnapshot(result: SnapshotResult): string {
  return yamlLines(result)
    .map((line) => line.text)
    .join('\n')
}

/**
 * `mode: 'full'`: the YAML of the engine's `'ai'` tree, every element and all the text, within
 * `maxTokens` by the v1 format's cap (snapshot-cap.ts). It fits on most single sections and on
 * few whole pages (9k tokens at the median of 31 live pages, 52k at most), so the cap is what
 * keeps one careless call from costing what the v1 format saves. When it fits, the text is
 * `renderSnapshot`'s byte for byte.
 */
export function renderFullSnapshot(
  result: SnapshotResult,
  maxTokens: number,
): string {
  return cap(
    null,
    printed(yamlLines(result)),
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
    const line: Printed = { text, ref: node?.ref !== undefined }
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
