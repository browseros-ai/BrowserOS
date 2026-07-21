import { describe, expect, test } from 'bun:test'
import { createPreviewHandlers } from '../../../src/api/http/handlers/previews'
import { httpTestApp } from './fixtures'

describe('preview HTTP handlers', () => {
  test('serves fresh JPEG bytes without caching', async () => {
    const response = await httpTestApp({
      previews: createPreviewHandlers({
        getSessionPreview: () => ({
          bytes: new Uint8Array([0xff, 0xd8]),
          etag: 'fresh',
        }),
      }),
    }).request('/api/v1/sessions/session-live/preview')
    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toBe('image/jpeg')
    expect(response.headers.get('cache-control')).toBe('private, no-store')
    expect(response.headers.get('etag')).toBe('"fresh"')
    expect(new Uint8Array(await response.arrayBuffer())).toEqual(
      new Uint8Array([0xff, 0xd8]),
    )
  })

  test('collapses unresolved capture to preview_not_found', async () => {
    const response = await httpTestApp({
      previews: createPreviewHandlers({ getSessionPreview: () => null }),
    }).request('/api/v1/sessions/missing/preview')
    expect(response.status).toBe(404)
    expect(await response.json()).toEqual({
      code: 'preview_not_found',
      message: 'session preview not found',
    })
  })
})
