/**
 * Chromium drop events and their DataTransfer are built beside the pinned element.
 * This is upstream dom.ts's external-drop payload, after Rust's pointer checks;
 * only a cancelled dragover permits drop, and every event shares the same transfer.
 */
import type { Edit } from './protocol'

export function drop(
  element: Element,
  edit: Extract<Edit, { kind: 'drop' }>,
): string {
  const transfer = new DataTransfer()
  for (const payload of edit.payloads) {
    const bytes = Uint8Array.from(atob(payload.buffer), (c) => c.charCodeAt(0))
    transfer.items.add(
      new File([bytes], payload.name, {
        type: payload.mimeType,
        lastModified: payload.lastModifiedMs,
      }),
    )
  }
  for (const { mimeType, value } of edit.data) transfer.setData(mimeType, value)
  const event = (type: string) =>
    new DragEvent(type, {
      bubbles: true,
      cancelable: true,
      composed: true,
      clientX: edit.point.x,
      clientY: edit.point.y,
      dataTransfer: transfer,
    })
  element.dispatchEvent(event('dragenter'))
  const over = event('dragover')
  element.dispatchEvent(over)
  if (!over.defaultPrevented) {
    element.dispatchEvent(event('dragleave'))
    return 'not-accepted'
  }
  element.dispatchEvent(event('drop'))
  return 'accepted'
}
