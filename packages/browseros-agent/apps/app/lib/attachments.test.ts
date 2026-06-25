import { describe, expect, it } from 'bun:test'
import { stageAttachment } from './attachments'

function restoreGlobal(name: string, value: unknown) {
  if (value === undefined) {
    Reflect.deleteProperty(globalThis, name)
    return
  }
  Reflect.set(globalThis, name, value)
}

describe('stageAttachment', () => {
  it('uses the recompressed blob media type for large images', async () => {
    const originalCreateImageBitmap = Reflect.get(
      globalThis,
      'createImageBitmap',
    )
    const originalOffscreenCanvas = Reflect.get(globalThis, 'OffscreenCanvas')
    const originalHTMLCanvasElement = Reflect.get(
      globalThis,
      'HTMLCanvasElement',
    )

    class FakeOffscreenCanvas {
      width: number
      height: number

      constructor(width: number, height: number) {
        this.width = width
        this.height = height
      }

      getContext() {
        return {
          drawImage() {},
        }
      }

      async convertToBlob(options: { type?: string }) {
        return new Blob([new Uint8Array([9, 8, 7])], {
          type: options.type ?? 'image/jpeg',
        })
      }
    }

    try {
      Reflect.set(globalThis, 'createImageBitmap', async () => ({
        width: 4096,
        height: 2048,
        close() {},
      }))
      Reflect.set(globalThis, 'OffscreenCanvas', FakeOffscreenCanvas)
      Reflect.set(globalThis, 'HTMLCanvasElement', class HTMLCanvasElement {})

      const file = new File([new Uint8Array(2 * 1024 * 1024)], 'shot.png', {
        type: 'image/png',
      })

      const result = await stageAttachment(file)

      expect(result.ok).toBe(true)
      if (!result.ok) throw new Error(result.error.message)
      expect(result.attachment.mediaType).toBe('image/jpeg')
      expect(result.attachment.dataUrl).toStartWith('data:image/jpeg;base64,')
      expect(result.attachment.payload).toMatchObject({
        kind: 'image',
        mediaType: 'image/jpeg',
        dataUrl: result.attachment.dataUrl,
      })
    } finally {
      restoreGlobal('createImageBitmap', originalCreateImageBitmap)
      restoreGlobal('OffscreenCanvas', originalOffscreenCanvas)
      restoreGlobal('HTMLCanvasElement', originalHTMLCanvasElement)
    }
  })
})
