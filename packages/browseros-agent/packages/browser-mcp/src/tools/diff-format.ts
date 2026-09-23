import type { SnapshotDiff } from '@browseros/browser-core/core/snapshot/diff'
import { writeTempToolOutputFile } from './output-file'
import {
  estimateTextTokens,
  sliceTextByEstimatedTokens,
} from './token-estimate'
import { wrapUntrusted } from './trust-boundary'

const MAX_INLINE_DIFF_TOKENS = 10_000
const MAX_INLINE_EXCERPT_TOKENS = 5_000

export interface FormattedDiff {
  text: string
  structured?: Record<string, unknown>
}

/**
 * How much of a changed diff to render. Defaults to 'full' (the token-bounded
 * inline diff with spill-to-file). 'summary' returns only change counts; a
 * { maxChars } budget caps the inline diff to that many characters, spilling the
 * rest to a file (mirrors the evaluate maxChars control) (#2700).
 */
export type DiffDetail = 'full' | 'summary' | { maxChars: number }

/** Formats observer diffs for direct tools and automatic post-action readback. */
export async function formatDiffResult(
  diff: SnapshotDiff,
  origin: string,
  detail: DiffDetail = 'full',
): Promise<FormattedDiff> {
  if (!diff.changed) {
    return {
      text: 'no change since last snapshot',
      structured: { changed: false },
    }
  }

  const structured = {
    changed: true,
    added: diff.added,
    removed: diff.removed,
    ...(diff.lineDiffSkipped && { lineDiffSkipped: true }),
    ...(diff.urlChanged && {
      urlChanged: true,
      beforeUrl: diff.beforeUrl,
      afterUrl: diff.afterUrl,
    }),
  }
  if (detail === 'summary') {
    const counts = `${diff.added} added, ${diff.removed} removed`
    const text = diff.urlChanged
      ? `URL changed (${diff.beforeUrl ?? '?'} -> ${diff.afterUrl ?? '?'}); ${counts}. Take a snapshot for the current state.`
      : `changed: ${counts}. Take a snapshot to see details.`
    return { text, structured }
  }

  const diffText = diff.text || '(empty page)'

  if (diff.lineDiffSkipped) {
    return {
      text: `${diffText}\nTake a fresh snapshot for the current state.`,
      structured,
    }
  }

  const wrappedDiff = wrapUntrusted(diffText, origin)

  if (typeof detail === 'object') {
    return capInlineDiff(
      diff,
      diffText,
      wrappedDiff,
      origin,
      structured,
      detail.maxChars,
    )
  }

  const tokenEstimate = estimateTextTokens(wrappedDiff)

  if (tokenEstimate > MAX_INLINE_DIFF_TOKENS) {
    const excerpt = sliceTextByEstimatedTokens(
      diffText,
      MAX_INLINE_EXCERPT_TOKENS,
    )
    try {
      const path = await writeTempToolOutputFile({
        toolName: 'diff',
        extension: 'md',
        content: wrappedDiff,
      })
      const summary = diff.urlChanged
        ? `URL changed; full current snapshot is ${tokenEstimate} estimated tokens, over the ${MAX_INLINE_DIFF_TOKENS}-token inline limit, saved to: ${path}\nRead the file for the full current snapshot.`
        : `Diff is ${tokenEstimate} estimated tokens, over the ${MAX_INLINE_DIFF_TOKENS}-token inline limit, saved to: ${path}\nRead the file for the full diff.`
      return {
        text: [
          summary,
          `Showing the first ${MAX_INLINE_EXCERPT_TOKENS} estimated tokens inline:`,
          wrapUntrusted(excerpt, origin),
        ].join('\n'),
        structured: {
          ...structured,
          truncated: true,
          tokenEstimate,
          path,
          contentLength: wrappedDiff.length,
          writtenToFile: true,
        },
      }
    } catch (error) {
      const saveError = error instanceof Error ? error.message : String(error)
      const text = diff.urlChanged
        ? `URL changed; full current snapshot is ${tokenEstimate} estimated tokens, over the ${MAX_INLINE_DIFF_TOKENS}-token inline limit, but saving it to a BrowserOS output file failed: ${saveError}`
        : `Diff is ${tokenEstimate} estimated tokens, over the ${MAX_INLINE_DIFF_TOKENS}-token inline limit, but saving it to a BrowserOS output file failed: ${saveError}`
      return {
        text: [
          text,
          `Showing the first ${MAX_INLINE_EXCERPT_TOKENS} estimated tokens instead:`,
          wrapUntrusted(excerpt, origin),
        ].join('\n'),
        structured: {
          ...structured,
          truncated: true,
          tokenEstimate,
          contentLength: wrappedDiff.length,
          writtenToFile: false,
          outputWriteFailed: true,
          error: saveError,
        },
      }
    }
  }

  if (diff.urlChanged) {
    return {
      text: `URL changed; returning full current snapshot instead of a diff:\n${wrappedDiff}`,
      structured,
    }
  }

  return {
    text: wrappedDiff,
    structured,
  }
}

/**
 * Caps a changed diff to a caller-supplied character budget: returns it whole
 * when it fits, otherwise an inline excerpt plus the full diff written to a local
 * output file. Char-based to mirror the evaluate maxChars control (#2700). When the
 * action navigated, diff.text is the new page's snapshot, so the navigation notice
 * is preserved here just as the full and summary modes do (a truncated snapshot
 * must not read as an ordinary in-page diff).
 */
async function capInlineDiff(
  diff: SnapshotDiff,
  diffText: string,
  wrappedDiff: string,
  origin: string,
  structured: Record<string, unknown>,
  maxChars: number,
): Promise<FormattedDiff> {
  const navNote = diff.urlChanged
    ? `URL changed (${diff.beforeUrl ?? '?'} -> ${diff.afterUrl ?? '?'}); the content below is the new page's current snapshot, not an in-page diff.\n`
    : ''
  const noun = diff.urlChanged ? 'Snapshot' : 'Diff'
  const nounLower = diff.urlChanged ? 'snapshot' : 'diff'

  if (diffText.length <= maxChars) {
    return { text: `${navNote}${wrappedDiff}`, structured }
  }

  const excerpt = wrapUntrusted(diffText.slice(0, maxChars), origin)
  try {
    const path = await writeTempToolOutputFile({
      toolName: 'diff',
      extension: 'md',
      content: wrappedDiff,
    })
    return {
      text: [
        `${navNote}${noun} truncated at ${maxChars} chars. Full ${nounLower} (${wrappedDiff.length} chars) saved to: ${path}`,
        excerpt,
      ].join('\n'),
      structured: {
        ...structured,
        truncated: true,
        path,
        contentLength: wrappedDiff.length,
        writtenToFile: true,
      },
    }
  } catch (error) {
    const saveError = error instanceof Error ? error.message : String(error)
    return {
      text: [
        `${navNote}${noun} truncated at ${maxChars} chars. Full ${nounLower} (${wrappedDiff.length} chars) could not be saved to a BrowserOS output file: ${saveError}`,
        excerpt,
      ].join('\n'),
      structured: {
        ...structured,
        truncated: true,
        contentLength: wrappedDiff.length,
        writtenToFile: false,
        outputWriteFailed: true,
        error: saveError,
      },
    }
  }
}
