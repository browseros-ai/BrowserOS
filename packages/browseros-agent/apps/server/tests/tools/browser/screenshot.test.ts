import { describe, expect, it } from 'bun:test'
import { captureScreenshotWithAnnotations } from '../../../src/browser/core/screenshot'
import { RefMap } from '../../../src/browser/core/snapshot/refs'

function createRefs(): RefMap {
  const refs = new RefMap()
  refs.mint({
    backendNodeId: 101,
    role: 'button',
    name: 'Save',
    documentId: 'doc-1',
  })
  return refs
}

function createHarness(
  options: {
    rect?: { x: number; y: number; width: number; height: number }
    rejectCapture?: boolean
    onCapture?: (count: number) => Promise<void> | void
  } = {},
) {
  const events: string[] = []
  const expressions: string[] = []
  const objectGroups: string[] = []
  const captureParams: unknown[] = []
  const refs = createRefs()
  let captureCount = 0
  const pageSession = {
    DOM: {
      resolveNode: async ({
        backendNodeId,
        objectGroup,
      }: {
        backendNodeId: number
        objectGroup?: string
      }) => {
        events.push(`resolve:${backendNodeId}`)
        if (objectGroup) objectGroups.push(objectGroup)
        return { object: { objectId: `node-${backendNodeId}` } }
      },
    },
    Runtime: {
      callFunctionOn: async ({ objectId }: { objectId?: string }) => {
        events.push(`bounds:${objectId}`)
        return {
          result: {
            value: options.rect ?? {
              x: 10.6,
              y: 20.5,
              width: 40.2,
              height: 15.4,
            },
          },
        }
      },
      evaluate: async ({ expression }: { expression: string }) => {
        expressions.push(expression)
        if (expression.trim().startsWith('({x: window.scrollX')) {
          events.push('scroll')
          return { result: { value: { x: 5, y: 100 } } }
        }
        if (expression.trim().startsWith('({x:0,y:0,width:')) {
          events.push('viewport')
          return {
            result: { value: { x: 0, y: 0, width: 800, height: 600 } },
          }
        }
        if (expression.includes('createElement')) {
          events.push('inject')
        } else if (expression.includes('querySelectorAll')) {
          events.push('remove')
        } else {
          events.push('evaluate')
        }
        return { result: { value: true } }
      },
      releaseObjectGroup: async ({ objectGroup }: { objectGroup: string }) => {
        objectGroups.push(`release:${objectGroup}`)
        events.push('release')
      },
    },
    Page: {
      captureScreenshot: async (params: {
        captureBeyondViewport?: boolean
      }) => {
        captureParams.push(params)
        events.push(`capture:${params.captureBeyondViewport}`)
        captureCount += 1
        await options.onCapture?.(captureCount)
        if (options.rejectCapture) throw new Error('capture failed')
        return { data: 'png-data' }
      },
    },
  }
  const observer = {
    snapshot: async () => {
      events.push('snapshot')
      return {
        text: '- button "Save" [ref=e1]',
        refs,
        url: 'https://example.com',
      }
    },
    resolveRef: async (ref: string) => {
      events.push(`ref:${ref}`)
      return { session: pageSession, backendNodeId: 101 }
    },
  }

  return {
    captureParams,
    events,
    expressions,
    objectGroups,
    observer,
    pageSession,
  }
}

function deferred() {
  let resolve = () => {}
  const promise = new Promise<void>((done) => {
    resolve = done
  })
  return { promise, resolve }
}

describe('captureScreenshotWithAnnotations', () => {
  it('defaults through the annotation lifecycle and returns annotation metadata', async () => {
    const harness = createHarness()

    const result = await captureScreenshotWithAnnotations({
      pageSession: harness.pageSession as never,
      observer: harness.observer as never,
      options: { format: 'png', fullPage: false },
    })

    expect(harness.events).toEqual([
      'viewport',
      'snapshot',
      'ref:e1',
      'resolve:101',
      'bounds:node-101',
      'inject',
      'capture:false',
      'remove',
      'release',
    ])
    expect(harness.expressions[1]).toContain(
      'data-browseros-screenshot-annotation',
    )
    expect(harness.expressions[1]).not.toContain('getElementById')
    expect(harness.expressions[1]).toContain(
      'var useDocumentSpaceLabels = false;',
    )
    expect(harness.expressions[1]).toContain(
      'var labelAnchor = useDocumentSpaceLabels ? dy : it.y;',
    )
    expect(harness.expressions[1]).toContain('"number":1')
    expect(result).toEqual({
      data: 'png-data',
      mimeType: 'image/png',
      annotations: [
        {
          ref: 'e1',
          number: 1,
          role: 'button',
          name: 'Save',
          box: { x: 11, y: 21, width: 40, height: 15 },
        },
      ],
    })
  })

  it('captures without snapshot or overlay work when annotations are disabled', async () => {
    const harness = createHarness()
    harness.observer.snapshot = async () => {
      throw new Error('snapshot should not run')
    }

    const result = await captureScreenshotWithAnnotations({
      pageSession: harness.pageSession as never,
      observer: harness.observer as never,
      options: { format: 'png', fullPage: true, annotate: false },
    })

    expect(harness.events).toEqual(['capture:true'])
    expect(result).toEqual({
      data: 'png-data',
      mimeType: 'image/png',
      annotations: [],
    })
  })

  it('removes the injected overlay when screenshot capture fails', async () => {
    const harness = createHarness({ rejectCapture: true })

    await expect(
      captureScreenshotWithAnnotations({
        pageSession: harness.pageSession as never,
        observer: harness.observer as never,
        options: { format: 'png', fullPage: false, annotate: true },
      }),
    ).rejects.toThrow('capture failed')

    expect(harness.events).toEqual([
      'viewport',
      'snapshot',
      'ref:e1',
      'resolve:101',
      'bounds:node-101',
      'inject',
      'capture:false',
      'remove',
      'release',
    ])
  })

  it('projects full-page annotation metadata into document coordinates', async () => {
    const harness = createHarness()

    const result = await captureScreenshotWithAnnotations({
      pageSession: harness.pageSession as never,
      observer: harness.observer as never,
      options: { format: 'png', fullPage: true, annotate: true },
    })

    expect(harness.events).toEqual([
      'snapshot',
      'ref:e1',
      'resolve:101',
      'bounds:node-101',
      'scroll',
      'inject',
      'capture:true',
      'remove',
      'release',
    ])
    expect(result.annotations[0]?.box).toEqual({
      x: 16,
      y: 121,
      width: 40,
      height: 15,
    })
  })

  it('clips viewport annotations to the visible screenshot area', async () => {
    const harness = createHarness({
      rect: { x: -5, y: 10, width: 20, height: 20 },
    })

    const result = await captureScreenshotWithAnnotations({
      pageSession: harness.pageSession as never,
      observer: harness.observer as never,
      options: { format: 'png', fullPage: false },
    })

    expect(result.annotations[0]?.box).toEqual({
      x: 0,
      y: 10,
      width: 15,
      height: 20,
    })
  })

  it('scales viewport annotation metadata for clipped screenshots', async () => {
    const harness = createHarness()
    const clip = { x: 0, y: 0, width: 800, height: 600, scale: 0.5 }

    const result = await captureScreenshotWithAnnotations({
      pageSession: harness.pageSession as never,
      observer: harness.observer as never,
      options: { format: 'jpeg', fullPage: false, clip },
    })

    expect(harness.captureParams[0]).toEqual({
      format: 'jpeg',
      fromSurface: true,
      captureBeyondViewport: false,
      clip,
    })
    expect(result.annotations[0]?.box).toEqual({
      x: 5,
      y: 10,
      width: 20,
      height: 8,
    })
  })

  it('serializes concurrent annotated captures on the same page session', async () => {
    const firstCaptureStarted = deferred()
    const releaseFirstCapture = deferred()
    const harness = createHarness({
      onCapture: async (count) => {
        if (count === 1) {
          firstCaptureStarted.resolve()
          await releaseFirstCapture.promise
        }
      },
    })
    const input = {
      pageSession: harness.pageSession as never,
      observer: harness.observer as never,
      options: { format: 'png' as const, fullPage: false },
    }

    const first = captureScreenshotWithAnnotations(input)
    await firstCaptureStarted.promise
    const second = captureScreenshotWithAnnotations(input)
    await Promise.resolve()

    expect(harness.events).toHaveLength(7)
    expect(harness.events.at(-1)).toBe('capture:false')

    releaseFirstCapture.resolve()
    await Promise.all([first, second])

    expect(harness.events).toEqual([
      'viewport',
      'snapshot',
      'ref:e1',
      'resolve:101',
      'bounds:node-101',
      'inject',
      'capture:false',
      'remove',
      'release',
      'viewport',
      'snapshot',
      'ref:e1',
      'resolve:101',
      'bounds:node-101',
      'inject',
      'capture:false',
      'remove',
      'release',
    ])
    const resolveGroups = harness.objectGroups.filter(
      (group) => !group.startsWith('release:'),
    )
    const releaseGroups = harness.objectGroups
      .filter((group) => group.startsWith('release:'))
      .map((group) => group.replace(/^release:/, ''))
    expect(resolveGroups).toHaveLength(2)
    expect(releaseGroups).toEqual(resolveGroups)
    expect(resolveGroups[0]).not.toBe(resolveGroups[1])
  })

  it('keeps plain screenshots out of an in-flight annotated overlay window', async () => {
    const firstCaptureStarted = deferred()
    const releaseFirstCapture = deferred()
    const harness = createHarness({
      onCapture: async (count) => {
        if (count === 1) {
          firstCaptureStarted.resolve()
          await releaseFirstCapture.promise
        }
      },
    })

    const annotated = captureScreenshotWithAnnotations({
      pageSession: harness.pageSession as never,
      observer: harness.observer as never,
      options: { format: 'png', fullPage: false },
    })
    await firstCaptureStarted.promise
    const plain = captureScreenshotWithAnnotations({
      pageSession: harness.pageSession as never,
      observer: harness.observer as never,
      options: { format: 'png', fullPage: false, annotate: false },
    })
    await Promise.resolve()

    expect(harness.events).toHaveLength(7)
    expect(harness.events.at(-1)).toBe('capture:false')

    releaseFirstCapture.resolve()
    await Promise.all([annotated, plain])

    expect(harness.events).toEqual([
      'viewport',
      'snapshot',
      'ref:e1',
      'resolve:101',
      'bounds:node-101',
      'inject',
      'capture:false',
      'remove',
      'release',
      'capture:false',
    ])
  })
})
