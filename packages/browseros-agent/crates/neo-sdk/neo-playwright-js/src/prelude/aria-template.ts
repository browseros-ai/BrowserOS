/**
 * `toMatchAriaSnapshot`'s template on its way to the page. Playwright splits the matcher in
 * three: the test runner trims the YAML (matchers/toMatchAriaSnapshot.ts), the server parses
 * it (`Frame.expect`, server/frames.ts), and the page matches the parsed template
 * (`matchesExpectAriaTemplate`). Neo's prelude does the first two here, with the vendored
 * parser and the yaml release Playwright bundles, so a template parses (and fails) as in
 * Playwright. The parsed template is plain JSON by upstream's design, and reaches the probe in
 * the op's params unchanged.
 */
import { LineCounter, parseDocument, Scalar, YAMLMap, YAMLSeq } from 'yaml'
import {
  type AriaTemplateNode,
  parseAriaSnapshotUnsafe,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'

/** The library parseAriaSnapshot takes; the build resolves `yaml` to third_party/yaml. */
const YAML = { LineCounter, parseDocument, Scalar, YAMLMap, YAMLSeq }

/**
 * The template's parse: the tree the page matches, or the message the server fails the
 * matcher with (an `ExpectError` whose message becomes the failure's `Error:` line).
 */
export type ParsedTemplate = { template: AriaTemplateNode } | { error: string }

export function parseTemplate(text: string): ParsedTemplate {
  try {
    return { template: parseAriaSnapshotUnsafe(YAML, text) }
  } catch (error) {
    return { error: error instanceof Error ? error.message : String(error) }
  }
}

/**
 * Upstream's `unshift`: drops blank lines and the indent every line shares, so a template
 * written inside an indented template literal reads from column 0.
 */
export function unshift(snapshot: string): string {
  const lines = snapshot.split('\n')
  let whitespacePrefixLength = 100
  for (const line of lines) {
    if (!line.trim()) continue
    const match = line.match(/^(\s*)/)
    if (match?.[1] !== undefined && match[1].length < whitespacePrefixLength)
      whitespacePrefixLength = match[1].length
  }
  return lines
    .filter((t) => t.trim())
    .map((line) => line.substring(whitespacePrefixLength))
    .join('\n')
}
