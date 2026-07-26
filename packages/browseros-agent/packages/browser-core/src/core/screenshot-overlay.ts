import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'

const ANNOTATION_OVERLAY_ATTR = 'data-browseros-screenshot-annotation'

export interface OverlayAnnotation {
  number: number
  rect: {
    x: number
    y: number
    width: number
    height: number
  }
}

export function createOverlayToken(): string {
  return `browseros-${Date.now()}-${Math.random().toString(36).slice(2)}`
}

export async function injectAnnotationOverlay(
  session: ProtocolApi,
  token: string,
  fullPage: boolean,
  annotations: OverlayAnnotation[],
  scroll?: { x: number; y: number },
): Promise<void> {
  const items = JSON.stringify(
    annotations.map((annotation) => ({
      number: annotation.number,
      x: round(annotation.rect.x),
      y: round(annotation.rect.y),
      width: round(annotation.rect.width),
      height: round(annotation.rect.height),
    })),
  )
  const overlayAttr = JSON.stringify(ANNOTATION_OVERLAY_ATTR)
  const overlayToken = JSON.stringify(token)
  const useDocumentSpaceLabels = JSON.stringify(fullPage)
  const scrollData = JSON.stringify(scroll ?? null)

  await session.Runtime.evaluate({
    expression: `(() => {
      var items = ${items};
      var attr = ${overlayAttr};
      var token = ${overlayToken};
      var useDocumentSpaceLabels = ${useDocumentSpaceLabels};
      var scroll = ${scrollData};
      var sx = scroll && typeof scroll.x === 'number' ? scroll.x : window.scrollX || 0;
      var sy = scroll && typeof scroll.y === 'number' ? scroll.y : window.scrollY || 0;
      var c = document.createElement('div');
      c.setAttribute(attr, token);
      c.style.cssText = 'position:absolute;top:0;left:0;width:0;height:0;pointer-events:none;z-index:2147483647;';
      for (var i = 0; i < items.length; i++) {
        var it = items[i];
        var dx = it.x + sx;
        var dy = it.y + sy;
        var b = document.createElement('div');
        b.style.cssText = 'position:absolute;left:' + dx + 'px;top:' + dy + 'px;width:' + it.width + 'px;height:' + it.height + 'px;border:2px solid rgba(255,0,0,0.8);box-sizing:border-box;pointer-events:none;';
        var l = document.createElement('div');
        l.textContent = String(it.number);
        var labelAnchor = useDocumentSpaceLabels ? dy : it.y;
        var labelTop = labelAnchor < 14 ? '2px' : '-14px';
        l.style.cssText = 'position:absolute;top:' + labelTop + ';left:-2px;background:rgba(255,0,0,0.9);color:#fff;font:bold 11px/14px monospace;padding:0 4px;border-radius:2px;white-space:nowrap;';
        b.appendChild(l);
        c.appendChild(b);
      }
      document.documentElement.appendChild(c);
      return true;
    })()`,
    returnByValue: true,
    awaitPromise: false,
  })
}

export async function removeAnnotationOverlay(
  session: ProtocolApi,
  token: string,
): Promise<void> {
  const overlayAttr = JSON.stringify(ANNOTATION_OVERLAY_ATTR)
  const overlayToken = JSON.stringify(token)
  await session.Runtime.evaluate({
    expression: `(() => {
      var attr = ${overlayAttr};
      var token = ${overlayToken};
      var existing = document.querySelectorAll('[' + attr + ']');
      for (var i = 0; i < existing.length; i++) {
        if (existing[i].getAttribute(attr) === token) existing[i].remove();
      }
      return true;
    })()`,
    returnByValue: true,
    awaitPromise: false,
  })
}

function round(value: number): number {
  return Math.round(value)
}
