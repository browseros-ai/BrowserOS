import type { Viewport } from '@browseros/cdp-protocol/domains/page'
import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { Observer } from './observer/observer'
import { frameDepth } from './screenshot-frame'
import {
  clipAnnotations,
  parseRect,
  projectAnnotations,
  type RawAnnotation,
  type Rect,
  readScrollOffsets,
  readViewportRect,
} from './screenshot-geometry'
import {
  createOverlayToken,
  injectAnnotationOverlay,
  removeAnnotationOverlay,
} from './screenshot-overlay'
import { runExclusiveScreenshotCapture } from './screenshot-queue'
import type { RefEntry } from './snapshot/refs'

export type ScreenshotFormat = 'png' | 'jpeg' | 'webp'

export interface ScreenshotCaptureOptions {
  format?: ScreenshotFormat
  quality?: number
  fullPage?: boolean
  annotate?: boolean
  clip?: Viewport
}

export interface ScreenshotAnnotationBox {
  x: number
  y: number
  width: number
  height: number
}

export interface ScreenshotAnnotation {
  ref: string
  number: number
  role: string
  name?: string
  box: ScreenshotAnnotationBox
}

export interface ScreenshotCaptureResult {
  data: string
  mimeType: string
  annotations: ScreenshotAnnotation[]
}

export interface CaptureInput {
  pageSession: ProtocolApi
  observer: Observer
  options: ScreenshotCaptureOptions
}

/** Captures a screenshot, optionally painting current snapshot refs into a temporary page overlay. */
export async function captureScreenshotWithAnnotations({
  pageSession,
  observer,
  options,
}: CaptureInput): Promise<ScreenshotCaptureResult> {
  if ((options.annotate ?? false) === false) {
    return runExclusiveScreenshotCapture(pageSession, () =>
      capturePlainScreenshot(pageSession, options),
    )
  }

  return runExclusiveScreenshotCapture(pageSession, () =>
    captureAnnotatedScreenshot({ pageSession, observer, options }),
  )
}

async function capturePlainScreenshot(
  pageSession: ProtocolApi,
  options: ScreenshotCaptureOptions,
): Promise<ScreenshotCaptureResult> {
  const format = options.format ?? 'png'
  const fullPage = options.fullPage ?? false

  const result = await pageSession.Page.captureScreenshot({
    format,
    fromSurface: true,
    ...(format !== 'png' &&
      options.quality !== undefined && { quality: options.quality }),
    captureBeyondViewport: fullPage,
    ...(!fullPage && options.clip && { clip: options.clip }),
  })

  return {
    data: result.data,
    mimeType: `image/${format}`,
    annotations: [],
  }
}

async function captureAnnotatedScreenshot({
  pageSession,
  observer,
  options,
}: CaptureInput): Promise<ScreenshotCaptureResult> {
  const format = options.format ?? 'png'
  const fullPage = options.fullPage ?? false
  let annotations: RawAnnotation[] = []
  let scroll: { x: number; y: number } | undefined
  let overlayInjected = false
  const overlayToken = createOverlayToken()
  const objectGroup = overlayToken
  const objectSessions = new Set<ProtocolApi>()

  try {
    const captureArea = fullPage
      ? undefined
      : options.clip
        ? { x: 0, y: 0, width: options.clip.width, height: options.clip.height }
        : await readViewportRect(pageSession).catch(() => undefined)
    annotations = clipAnnotations(
      await collectAnnotations(
        pageSession,
        observer,
        objectGroup,
        objectSessions,
      ),
      captureArea,
    )
    if (annotations.length > 0) {
      scroll = fullPage
        ? await readScrollOffsets(pageSession).catch(() => undefined)
        : undefined
      await injectAnnotationOverlay(
        pageSession,
        overlayToken,
        fullPage,
        annotations,
        scroll,
      )
      overlayInjected = true
    }

    const result = await pageSession.Page.captureScreenshot({
      format,
      fromSurface: true,
      ...(format !== 'png' &&
        options.quality !== undefined && { quality: options.quality }),
      captureBeyondViewport: fullPage,
      ...(!fullPage && options.clip && { clip: options.clip }),
    })
    return {
      data: result.data,
      mimeType: `image/${format}`,
      annotations: projectAnnotations(
        annotations,
        scroll,
        fullPage ? undefined : options.clip?.scale,
      ),
    }
  } finally {
    if (overlayInjected) {
      await removeAnnotationOverlay(pageSession, overlayToken).catch(() => {})
    }
    await releaseObjectGroup(objectSessions, objectGroup)
  }
}

async function collectAnnotations(
  pageSession: ProtocolApi,
  observer: Observer,
  objectGroup: string,
  objectSessions: Set<ProtocolApi>,
): Promise<RawAnnotation[]> {
  const snapshot = await observer.snapshot()
  const entries = [...snapshot.refs.byRef.values()].sort(
    (left, right) => annotationNumber(left.ref) - annotationNumber(right.ref),
  )
  const annotations = await Promise.all(
    entries.map((entry) =>
      collectAnnotation(
        pageSession,
        observer,
        objectGroup,
        objectSessions,
        entry,
      ),
    ),
  )
  return annotations.filter((item): item is RawAnnotation => item !== undefined)
}

async function collectAnnotation(
  pageSession: ProtocolApi,
  observer: Observer,
  objectGroup: string,
  objectSessions: Set<ProtocolApi>,
  entry: RefEntry,
): Promise<RawAnnotation | undefined> {
  try {
    const resolved = await observer.resolveRef(entry.ref)
    const localRect = await readElementRect(
      resolved.session,
      resolved.backendNodeId,
      objectGroup,
      objectSessions,
    )
    if (!localRect) return undefined

    const rect =
      entry.frameId === undefined
        ? localRect
        : await projectFrameRect(
            pageSession,
            objectGroup,
            objectSessions,
            entry.frameId,
            localRect,
          )
    if (!rect) return undefined

    return {
      ref: entry.ref,
      number: annotationNumber(entry.ref),
      role: entry.role,
      ...(entry.name && { name: entry.name }),
      rect,
    }
  } catch {
    return undefined
  }
}

async function projectFrameRect(
  pageSession: ProtocolApi,
  objectGroup: string,
  objectSessions: Set<ProtocolApi>,
  frameId: string,
  rect: Rect,
): Promise<Rect | undefined> {
  try {
    if ((await frameDepth(pageSession, frameId)) !== 1) return undefined
    const owner = await pageSession.DOM.getFrameOwner({ frameId })
    const offset = await readFrameContentOffset(
      pageSession,
      owner.backendNodeId,
      objectGroup,
      objectSessions,
    )
    if (!offset) return undefined
    return {
      x: offset.x + rect.x,
      y: offset.y + rect.y,
      width: rect.width,
      height: rect.height,
    }
  } catch {
    return undefined
  }
}

async function readElementRect(
  session: ProtocolApi,
  backendNodeId: number,
  objectGroup: string,
  objectSessions: Set<ProtocolApi>,
): Promise<Rect | undefined> {
  const objectId = await resolveObjectId(
    session,
    backendNodeId,
    objectGroup,
    objectSessions,
  )
  if (!objectId) return undefined

  const result = await session.Runtime.callFunctionOn({
    functionDeclaration:
      'function(){var r=this.getBoundingClientRect();return{x:r.x,y:r.y,width:r.width,height:r.height}}',
    objectId,
    returnByValue: true,
  })
  const rect = parseRect(result.result?.value)
  if (!rect || rect.width <= 0 || rect.height <= 0) return undefined
  return rect
}

async function readFrameContentOffset(
  session: ProtocolApi,
  backendNodeId: number,
  objectGroup: string,
  objectSessions: Set<ProtocolApi>,
): Promise<{ x: number; y: number } | undefined> {
  const objectId = await resolveObjectId(
    session,
    backendNodeId,
    objectGroup,
    objectSessions,
  )
  if (!objectId) return undefined

  const result = await session.Runtime.callFunctionOn({
    functionDeclaration:
      'function(){var r=this.getBoundingClientRect();return{x:r.x+(this.clientLeft||0),y:r.y+(this.clientTop||0)}}',
    objectId,
    returnByValue: true,
  })
  const value = result.result?.value
  if (!isRecord(value)) return undefined
  const x = value.x
  const y = value.y
  if (typeof x !== 'number' || typeof y !== 'number') return undefined
  if (!Number.isFinite(x) || !Number.isFinite(y)) return undefined
  return { x, y }
}

async function resolveObjectId(
  session: ProtocolApi,
  backendNodeId: number,
  objectGroup: string,
  objectSessions: Set<ProtocolApi>,
): Promise<string | undefined> {
  try {
    const resolved = await session.DOM.resolveNode({
      backendNodeId,
      objectGroup,
    })
    const objectId = resolved.object?.objectId
    if (objectId) objectSessions.add(session)
    return objectId
  } catch {
    return undefined
  }
}

async function releaseObjectGroup(
  sessions: Set<ProtocolApi>,
  objectGroup: string,
): Promise<void> {
  await Promise.all(
    [...sessions].map((session) =>
      session.Runtime.releaseObjectGroup({ objectGroup }).catch(() => {}),
    ),
  )
}

function annotationNumber(ref: string): number {
  const number = Number.parseInt(ref.replace(/^e/, ''), 10)
  return Number.isFinite(number) ? number : 0
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}
