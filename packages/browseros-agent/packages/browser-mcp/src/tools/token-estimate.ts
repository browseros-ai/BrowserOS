const APPROX_CHARS_PER_TOKEN = 3

/** Estimates plain text tokens with the same chars/3 heuristic used by compaction. */
export function estimateTextTokens(text: string): number {
  return Math.ceil(text.length / APPROX_CHARS_PER_TOKEN)
}

/** Returns the longest prefix that stays within the estimated token limit. */
export function sliceTextByEstimatedTokens(
  text: string,
  maxTokens: number,
): string {
  if (estimateTextTokens(text) <= maxTokens) return text

  let low = 0
  let high = text.length
  while (low < high) {
    const mid = Math.floor((low + high + 1) / 2)
    if (estimateTextTokens(text.slice(0, mid)) <= maxTokens) {
      low = mid
    } else {
      high = mid - 1
    }
  }

  return text.slice(0, low)
}
