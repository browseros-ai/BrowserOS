/** Contract client and the shared UI stream decoder. */

export type { UiMessageChunk } from '@browseros/server-api'
export { ServerApiClient } from './client'
export { ServerApiError } from './errors'
export type { components, operations, paths } from './generated/openapi'
export { readSseData, readUiMessageStream } from './stream'
