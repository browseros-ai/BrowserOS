import type { Handler } from 'hono'
import { sessionVisualService } from '../../../services/session-visuals'
import { apiError } from '../errors'
import type { RequestContextEnv } from '../request-context'
import { type BinaryAsset, binaryResponse } from '../responses'

export interface PreviewHandlerDependencies {
  getSessionPreview(
    sessionId: string,
  ): BinaryAsset | null | Promise<BinaryAsset | null>
}

export interface PreviewHandlers {
  get: Handler<RequestContextEnv>
}

export function createPreviewHandlers(
  dependencies: PreviewHandlerDependencies,
): PreviewHandlers {
  return {
    get: async (c) => {
      const asset = await dependencies.getSessionPreview(
        c.req.param('sessionId') ?? '',
      )
      if (!asset) {
        return apiError(
          c,
          404,
          'preview_not_found',
          'session preview not found',
        )
      }
      return binaryResponse(asset, 'private, no-store')
    },
  }
}

export const productionPreviewHandlers = createPreviewHandlers({
  async getSessionPreview(sessionId) {
    const bytes = await sessionVisualService.capture(sessionId)
    return bytes ? { bytes } : null
  },
})
