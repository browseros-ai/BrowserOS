export interface ToolResultImageSource {
  content?: unknown
  structuredContent?: unknown
}

/** Extracts base64 image data from MCP image content, with a legacy structured fallback. */
export function extractToolResultImageData(
  result: ToolResultImageSource,
): string | null {
  return (
    imageDataFromContent(result.content) ??
    imageDataFromStructuredContent(result.structuredContent)
  )
}

function imageDataFromContent(content: unknown): string | null {
  if (!Array.isArray(content)) return null
  for (const item of content) {
    if (!item || typeof item !== 'object') continue
    const block = item as Record<string, unknown>
    if (block.type !== 'image') continue
    if (typeof block.data === 'string' && block.data.length > 0) {
      return block.data
    }
  }
  return null
}

function imageDataFromStructuredContent(
  structuredContent: unknown,
): string | null {
  if (!structuredContent || typeof structuredContent !== 'object') return null
  const image = (structuredContent as Record<string, unknown>).image
  return typeof image === 'string' && image.length > 0 ? image : null
}
