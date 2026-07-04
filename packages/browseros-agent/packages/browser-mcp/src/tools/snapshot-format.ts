import { writeTempToolOutputFile } from './output-file'
import {
  estimateTextTokens,
  sliceTextByEstimatedTokens,
} from './token-estimate'
import { wrapUntrusted } from './trust-boundary'

const LARGE_SNAPSHOT_TOKEN_THRESHOLD = 15_000
const MAX_INLINE_EXCERPT_TOKENS = 5_000

export interface FormattedSnapshot {
  text: string
  structured?: Record<string, unknown>
}

/** Formats page snapshots for direct tools and automatic post-action readback. */
export async function formatSnapshotResult(
  snapshot: string,
  origin: string,
): Promise<FormattedSnapshot> {
  const snapshotText = snapshot || '(empty page)'
  const wrappedSnapshot = wrapUntrusted(snapshotText, origin)
  const contentLength = wrappedSnapshot.length
  const tokenEstimate = estimateTextTokens(wrappedSnapshot)

  if (tokenEstimate > LARGE_SNAPSHOT_TOKEN_THRESHOLD) {
    const excerpt = sliceTextByEstimatedTokens(
      snapshotText,
      MAX_INLINE_EXCERPT_TOKENS,
    )
    try {
      const path = await writeTempToolOutputFile({
        toolName: 'snapshot',
        extension: 'md',
        content: wrappedSnapshot,
      })

      return {
        text: [
          `Large snapshot (${tokenEstimate} estimated tokens, ${contentLength} chars) saved to: ${path}`,
          'Read the file for the full snapshot and refs.',
          `Showing the first ${MAX_INLINE_EXCERPT_TOKENS} estimated tokens inline:`,
          wrapUntrusted(excerpt, origin),
        ].join('\n'),
        structured: {
          path,
          contentLength,
          tokenEstimate,
          writtenToFile: true,
        },
      }
    } catch (error) {
      const saveError = error instanceof Error ? error.message : String(error)
      return {
        text: [
          `Large snapshot (${tokenEstimate} estimated tokens, ${contentLength} chars) could not be saved to a BrowserOS output file: ${saveError}`,
          `Showing the first ${MAX_INLINE_EXCERPT_TOKENS} estimated tokens instead:`,
          wrapUntrusted(excerpt, origin),
        ].join('\n'),
        structured: {
          contentLength,
          tokenEstimate,
          writtenToFile: false,
          outputWriteFailed: true,
          error: saveError,
        },
      }
    }
  }

  return {
    text: wrappedSnapshot,
    structured: { contentLength, tokenEstimate, writtenToFile: false },
  }
}
