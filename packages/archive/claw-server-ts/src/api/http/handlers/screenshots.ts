import type { SessionScreenshotList } from '@browseros/claw-api'
import type { Handler } from 'hono'
import { identityService } from '../../../lib/mcp-session'
import {
  listSessionScreenshots as listStoredSessionScreenshots,
  readSessionScreenshot,
} from '../../../services/screenshots'
import { getTask } from '../../../services/tasks'
import { apiError } from '../errors'
import type { RequestContextEnv } from '../request-context'
import { type BinaryAsset, binaryResponse } from '../responses'
import { positiveInteger } from '../validation'

export interface ScreenshotHandlerDependencies {
  listSessionScreenshots(
    sessionId: string,
  ): SessionScreenshotList | null | Promise<SessionScreenshotList | null>
  getSessionScreenshot(
    sessionId: string,
    screenshotId: number,
  ): BinaryAsset | null | Promise<BinaryAsset | null>
}

export interface ScreenshotHandlers {
  list: Handler<RequestContextEnv>
  get: Handler<RequestContextEnv>
}

export function createScreenshotHandlers(
  dependencies: ScreenshotHandlerDependencies,
): ScreenshotHandlers {
  return {
    list: async (c) => {
      const screenshots = await dependencies.listSessionScreenshots(
        c.req.param('sessionId') ?? '',
      )
      if (!screenshots) {
        return apiError(c, 404, 'session_not_found', 'session not found')
      }
      return c.json(screenshots)
    },
    get: async (c) => {
      const screenshotId = positiveInteger(c.req.param('screenshotId') ?? '')
      if (screenshotId === null) {
        return apiError(
          c,
          400,
          'invalid_request',
          'screenshotId must be positive',
        )
      }
      const asset = await dependencies.getSessionScreenshot(
        c.req.param('sessionId') ?? '',
        screenshotId,
      )
      if (!asset) {
        return apiError(
          c,
          404,
          'screenshot_not_found',
          'session screenshot not found',
        )
      }
      return binaryResponse(asset, 'public, max-age=31536000, immutable')
    },
  }
}

export const productionScreenshotHandlers = createScreenshotHandlers({
  listSessionScreenshots(sessionId) {
    return knownSession(sessionId)
      ? { items: listStoredSessionScreenshots(sessionId) }
      : null
  },
  getSessionScreenshot(sessionId, screenshotId) {
    const bytes = readSessionScreenshot(sessionId, screenshotId)
    return bytes ? { bytes } : null
  },
})

function knownSession(sessionId: string): boolean {
  return Boolean(identityService.getIdentity(sessionId) ?? getTask(sessionId))
}
