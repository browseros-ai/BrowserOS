import type { operations, paths } from './generated/openapi'

const previewPath =
  '/api/v1/sessions/{sessionId}/preview' as const satisfies keyof paths
const screenshotPath =
  '/api/v1/sessions/{sessionId}/screenshots/{screenshotId}' as const satisfies keyof paths

export type GetSessionPreviewRequest =
  operations['getSessionPreview']['parameters']['path'] &
    NonNullable<operations['getSessionPreview']['parameters']['query']>

export type GetSessionScreenshotRequest =
  operations['getSessionScreenshot']['parameters']['path']

export function buildSessionPreviewUrl(
  baseUrl: string,
  request: GetSessionPreviewRequest,
): string {
  const path = previewPath.replace(
    '{sessionId}',
    encodeURIComponent(request.sessionId),
  )
  const params = new URLSearchParams()
  if (request.refresh !== undefined) {
    params.set('refresh', request.refresh.toString())
  }
  if (request.browserTabId !== undefined) {
    params.set('browserTabId', request.browserTabId.toString())
  }
  const query = params.toString()
  return `${withoutTrailingSlash(baseUrl)}${path}${query ? `?${query}` : ''}`
}

export function buildSessionScreenshotUrl(
  baseUrl: string,
  request: GetSessionScreenshotRequest,
): string {
  const path = screenshotPath
    .replace('{sessionId}', encodeURIComponent(request.sessionId))
    .replace(
      '{screenshotId}',
      encodeURIComponent(request.screenshotId.toString()),
    )
  return `${withoutTrailingSlash(baseUrl)}${path}`
}

function withoutTrailingSlash(value: string): string {
  return value.replace(/\/+$/, '')
}
