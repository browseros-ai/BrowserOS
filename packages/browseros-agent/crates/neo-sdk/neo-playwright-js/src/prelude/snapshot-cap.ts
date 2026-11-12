/**
 * The size cap on a snapshot's text: the v1 format's (snapshot-v1.ts renders its lines) and
 * `mode: 'full'`'s, Playwright's own lines (snapshot.ts `renderFullSnapshot`). Each renderer
 * says which lines are sections and kept blocks; this cuts them the same way for both.
 *
 * Whole lines are kept from the top while they fit the limit, then one `# cut:` line says how
 * much was left out, how big the whole snapshot is, and which section to snapshot instead, then
 * every kept block (dialog, alert, ...) after the cut is printed whole: sites put dialogs at the
 * end of the document, and a cut must never hide one.
 */

/** One rendered line of a form, indented, with what the cap needs to know about it. */
export type Printed = {
  text: string
  /** It prints a `[ref=…]`. */
  ref: boolean
  /** A section line with content: what the cut line can offer to snapshot instead. */
  section?: Section
  /** The first line of a kept block: the index after its last line. */
  keptEnd?: number
}

export type Section = {
  ref: string
  /**
   * Its role, and its quoted name when it has one: `navigation "Footer"`. The cut line wraps it
   * as `the navigation "Footer" section`, since a bare role ("all of region") did not read as a
   * thing an agent could ask for.
   */
  label: string
  /** The index after its last line. */
  end: number
}

/** A token is counted as four characters of JS string length. */
export const CHARS_PER_TOKEN = 4

/**
 * `chars` as tokens rounded to the nearest 100, and at least 100, as the notice and the cut line
 * say sizes: "about 0 tokens" would misstate text that is there.
 */
export function aboutTokens(chars: number): number {
  return Math.max(100, Math.round(chars / CHARS_PER_TOKEN / 100) * 100)
}

/**
 * `notice` and `lines` within `limit` characters. The notice counts toward the limit; the cut
 * line and the kept blocks printed after it do not. `call` is the snapshot call the cut line
 * offers on a section's ref, in the caller's own mode, so following the offer gives that
 * section in the same format as the text that was cut.
 */
export function cap(
  notice: string | null,
  lines: readonly Printed[],
  limit: number,
  call = 'ariaSnapshot()',
): string {
  const head = notice === null ? [] : [notice]
  const whole = [...head, ...lines.map((line) => line.text)].join('\n')
  if (whole.length <= limit) return whole
  // Whole lines from the top while they fit; `used` counts the newline before each but the first.
  let used = -1
  let taken = 0
  for (const text of [...head, ...lines.map((line) => line.text)]) {
    if (used + 1 + text.length > limit) break
    used += 1 + text.length
    taken += 1
  }
  const top = head.slice(0, taken)
  let cut = Math.max(0, taken - head.length)
  // A kept block is never split: when the limit falls inside one, the cut goes before it. The
  // first such block is the outermost, since a nested block starts after its parent.
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
  let later: Section | undefined
  for (let index = cut; index < lines.length; ) {
    const line = lines[index]
    const end = line.keptEnd
    if (end !== undefined) {
      // At the left edge: no section is open after the cut line, so its old indentation would
      // point at nothing. A kept block nested in it prints once, inside it.
      const indent = line.text.length - line.text.trimStart().length
      for (const kept of lines.slice(index, end))
        blocks.push(kept.text.slice(indent))
      index = end
      continue
    }
    more += 1
    if (line.ref) withRefs += 1
    later ??= line.section
    index += 1
  }
  // Every line past the limit sits in a kept block, which prints whole anyway: nothing is cut,
  // so the text stays as it was, each block in its place.
  if (more === 0) return whole
  const size = `the whole snapshot is about ${aboutTokens(whole.length)} tokens`
  const open = openSection(lines, cut, limit)
  const offer = open
    ? `page.getByRef('${open.ref}').${call} shows all of the ${open.label} section. Or pass a larger maxTokens.`
    : later
      ? `page.getByRef('${later.ref}').${call} shows the ${later.label} section, which is not shown above. Or pass a larger maxTokens.`
      : 'Pass a larger maxTokens.'
  const cutLine = `# cut: ${more} more lines (${withRefs} with refs); ${size}. ${offer}`
  return [...top, ...linesBefore(lines, cut), cutLine, ...blocks].join('\n')
}

function linesBefore(lines: readonly Printed[], cut: number): string[] {
  return lines.slice(0, cut).map((line) => line.text)
}

/**
 * The outermost section open at the cut whose own lines fit the limit: its snapshot shows all
 * that the cut removed from it. A section larger than the limit is never offered, since its own
 * snapshot would be cut at almost the same place and the agent would go round in a loop.
 */
function openSection(
  lines: readonly Printed[],
  cut: number,
  limit: number,
): Section | undefined {
  for (let index = 0; index < cut; index++) {
    const section = lines[index].section
    if (section === undefined || section.end <= cut) continue
    const own = lines.slice(index, section.end).map((line) => line.text)
    if (own.join('\n').length <= limit) return section
  }
  return undefined
}
