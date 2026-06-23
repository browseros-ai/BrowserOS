import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

export interface RawAnnotation {
  ref: string
  number: number
  role: string
  name?: string
  rect: Rect
}

export async function readViewportRect(session: ProtocolApi): Promise<Rect> {
  const result = await session.Runtime.evaluate({
    expression:
      '({x:0,y:0,width:window.innerWidth||0,height:window.innerHeight||0})',
    returnByValue: true,
    awaitPromise: false,
  })
  return (
    parseRect(result.result?.value) ?? {
      x: 0,
      y: 0,
      width: 0,
      height: 0,
    }
  )
}

export async function readScrollOffsets(
  session: ProtocolApi,
): Promise<{ x: number; y: number }> {
  const result = await session.Runtime.evaluate({
    expression: '({x: window.scrollX || 0, y: window.scrollY || 0})',
    returnByValue: true,
    awaitPromise: false,
  })
  const value = result.result?.value
  if (!isRecord(value)) return { x: 0, y: 0 }
  const x =
    typeof value.x === 'number' && Number.isFinite(value.x) ? value.x : 0
  const y =
    typeof value.y === 'number' && Number.isFinite(value.y) ? value.y : 0
  return { x, y }
}

export function projectAnnotations(
  annotations: RawAnnotation[],
  scroll?: { x: number; y: number },
  scale = 1,
): Array<{
  ref: string
  number: number
  role: string
  name?: string
  box: Rect
}> {
  return annotations.map((annotation) => ({
    ref: annotation.ref,
    number: annotation.number,
    role: annotation.role,
    ...(annotation.name && { name: annotation.name }),
    box: {
      x: round((annotation.rect.x + (scroll?.x ?? 0)) * scale),
      y: round((annotation.rect.y + (scroll?.y ?? 0)) * scale),
      width: round(annotation.rect.width * scale),
      height: round(annotation.rect.height * scale),
    },
  }))
}

export function clipAnnotations(
  annotations: RawAnnotation[],
  captureArea: Rect | undefined,
): RawAnnotation[] {
  if (!captureArea || captureArea.width <= 0 || captureArea.height <= 0) {
    return annotations
  }
  return annotations.flatMap((annotation) => {
    const rect = intersectRects(annotation.rect, captureArea)
    return rect ? [{ ...annotation, rect }] : []
  })
}

export function parseRect(value: unknown): Rect | undefined {
  if (!isRecord(value)) return undefined
  const { x, y, width, height } = value
  if (
    typeof x !== 'number' ||
    typeof y !== 'number' ||
    typeof width !== 'number' ||
    typeof height !== 'number'
  ) {
    return undefined
  }
  if (
    !Number.isFinite(x) ||
    !Number.isFinite(y) ||
    !Number.isFinite(width) ||
    !Number.isFinite(height)
  ) {
    return undefined
  }
  return { x, y, width, height }
}

function intersectRects(left: Rect, right: Rect): Rect | undefined {
  const x1 = Math.max(left.x, right.x)
  const y1 = Math.max(left.y, right.y)
  const x2 = Math.min(left.x + left.width, right.x + right.width)
  const y2 = Math.min(left.y + left.height, right.y + right.height)
  if (x2 <= x1 || y2 <= y1) return undefined
  return { x: x1, y: y1, width: x2 - x1, height: y2 - y1 }
}

function round(value: number): number {
  return Math.round(value)
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}
