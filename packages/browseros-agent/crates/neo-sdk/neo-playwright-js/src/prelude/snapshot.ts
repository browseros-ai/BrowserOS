/**
 * Renders the snapshot ops' result as the YAML `ariaSnapshot()` returns.
 *
 * Rust stitches every frame's JSON, applies Neo's naming rule and masks it with every frame's
 * sweep; the text is made here, in the agent runtime, as upstream makes it in its server and
 * never in a page. Rendering in the root frame's world would copy cross-site frames' text into
 * the embedding site's renderer process. The YAML is the vendored renderer's, byte for byte;
 * Neo adds only one comment per frame it could not read, under that frame's iframe line:
 *
 *   - iframe [ref=e7]
 *     # partial: https://pay.example/form not read (timeout after 2000 ms)
 */
import {
  type AriaNodeJSON,
  type AriaSnapshotJSON,
  renderAriaSnapshotAsYaml,
} from './vendored'
import type { PartialFrame, SnapshotResult } from './wire'

function comment(partial: PartialFrame): string {
  return `# partial: ${partial.frame} not read (${partial.reason})`
}

/**
 * The iframe line for `ref`. `lineToNode` indexes the renderer's lines, which a key with a
 * raw newline would shift once split; the ref check catches that and falls back to a scan.
 */
function iframeLine(
  lines: string[],
  lineToNode: Map<number, AriaNodeJSON>,
  ref: string,
): number {
  const tag = `[ref=${ref}]`
  for (const [index, node] of lineToNode)
    if (node.ref === ref && lines[index]?.includes(tag)) return index
  return lines.findIndex((line) => /^\s*- /.test(line) && line.includes(tag))
}

/** The YAML for one snapshot op's `{ json, partial }`. */
export function renderSnapshot(result: SnapshotResult): string {
  const json: AriaSnapshotJSON = result.json
  const lineToNode = new Map<number, AriaNodeJSON>()
  const yaml = renderAriaSnapshotAsYaml(json, { lineToNode })
  if (result.partial.length === 0) return yaml
  const lines = yaml === '' ? [] : yaml.split('\n')
  const placed = result.partial.map((partial) => ({
    partial,
    line: iframeLine(lines, lineToNode, partial.ref),
  }))
  const unplaced: string[] = []
  // From the bottom up, so each insertion leaves the lines above it where they were.
  for (const { partial, line } of placed.sort((a, b) => b.line - a.line)) {
    if (line === -1) {
      unplaced.unshift(comment(partial))
      continue
    }
    const indent = /^\s*/.exec(lines[line] ?? '')?.[0] ?? ''
    lines.splice(line + 1, 0, `${indent}  ${comment(partial)}`)
  }
  return [...lines, ...unplaced].join('\n')
}
