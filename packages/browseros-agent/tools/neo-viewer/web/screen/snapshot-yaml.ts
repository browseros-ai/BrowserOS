/**
 * The YAML `ariaSnapshot({ mode: 'default' })` returned, rebuilt from Neo's recorded
 * `{ json, partial }`: the vendored Playwright renderer, plus one comment per frame Neo could not
 * read under that frame's iframe line (or at the end, when its iframe is not printed). This is
 * the prelude's `renderSnapshot` (neo-playwright-js/src/prelude/snapshot.ts) line for line. Its
 * module cannot be imported here: it pulls the vendored injected sources and their path aliases,
 * which only the prelude's build resolves. So its comment placement is mirrored: change both.
 */

import type { AriaNodeJSON } from '../../../../crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'
import { renderAriaSnapshotAsYaml } from '../../../../crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshotRenderer'

/** A frame Neo could not read, as the snapshot operation records it. */
export interface PartialFrame {
  ref: string
  frame: string
  reason: string
}

function comment(partial: PartialFrame): string {
  return `# partial: ${partial.frame} not read (${partial.reason})`
}

/** The iframe line for `ref`, as the prelude finds it. */
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

export function defaultSnapshotText(
  json: AriaNodeJSON[],
  partial: PartialFrame[],
): string {
  const lineToNode = new Map<number, AriaNodeJSON>()
  const yaml = renderAriaSnapshotAsYaml(json, { lineToNode })
  if (partial.length === 0) return yaml
  const lines = yaml === '' ? [] : yaml.split('\n')
  const placed = partial.map((frame) => ({
    frame,
    line: iframeLine(lines, lineToNode, frame.ref),
  }))
  const unplaced: string[] = []
  // From the bottom up, so each insertion leaves the lines above it where they were.
  for (const { frame, line } of placed.sort((a, b) => b.line - a.line)) {
    if (line === -1) {
      unplaced.unshift(comment(frame))
      continue
    }
    const indent = /^\s*/.exec(lines[line] ?? '')?.[0] ?? ''
    lines.splice(line + 1, 0, `${indent}  ${comment(frame)}`)
  }
  return [...lines, ...unplaced].join('\n')
}
