import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { Observer } from '../observer/observer'
import type { PageManager } from '../pages'
import {
  callOnElement,
  focusElement,
  getElementCenter,
  getInputValue,
  jsClick,
  scrollIntoView,
} from './geometry'
import { clearField, pressCombo, typeText } from './keyboard'
import {
  dispatchClick,
  dispatchDrag,
  dispatchHover,
  dispatchScroll,
  type MouseButton,
} from './mouse'

export interface ClickOptions {
  button?: MouseButton | string
  clickCount?: number
}

export type ScrollDirection = 'up' | 'down' | 'left' | 'right'

const SELECT_OPTION_FN = `function(val){
  for(var i=0;i<this.options.length;i++){
    if(this.options[i].value===val||this.options[i].textContent.trim()===val){
      this.selectedIndex=i;
      this.dispatchEvent(new Event('change',{bubbles:true}));
      return this.options[i].textContent.trim();
    }
  }
  return null;
}`

/**
 * The action layer over a page's refs. Mouse/scroll dispatch on the element's (frame) session
 * in that session's coordinates; keyboard dispatches on the page session against whatever the
 * focus moved to — the asymmetry CDP requires for OOPIFs (a no-op on the main frame).
 */
export class Input {
  constructor(
    private readonly observer: Observer,
    private readonly pages: PageManager,
    private readonly pageId: number,
  ) {}

  async click(ref: string, opts: ClickOptions = {}): Promise<void> {
    const { session, backendNodeId } = await this.observer.resolveRef(ref)
    await this.clickNode(session, backendNodeId, opts)
  }

  async clickBackendNode(
    backendNodeId: number,
    opts: ClickOptions = {},
  ): Promise<{ x: number; y: number } | undefined> {
    return this.withPageSessionRetry((session) =>
      this.clickNode(session, backendNodeId, opts),
    )
  }

  async clickAt(x: number, y: number, opts: ClickOptions = {}): Promise<void> {
    await this.withPageSessionRetry((session) =>
      dispatchClick(
        session,
        x,
        y,
        mouseButton(opts.button),
        opts.clickCount ?? 1,
        0,
      ),
    )
  }

  async hoverAt(x: number, y: number): Promise<void> {
    await this.withPageSessionRetry((session) => dispatchHover(session, x, y))
  }

  async typeAt(
    x: number,
    y: number,
    text: string,
    clear = false,
  ): Promise<void> {
    await this.withPageSessionRetry(async (session) => {
      await dispatchClick(session, x, y, 'left', 1, 0)
      if (clear) await clearField(session)
      await typeText(session, text)
    })
  }

  async dragAt(
    from: { x: number; y: number },
    to: { x: number; y: number },
  ): Promise<void> {
    await this.withPageSessionRetry((session) =>
      dispatchDrag(session, from, to),
    )
  }

  private async clickNode(
    session: ProtocolApi,
    backendNodeId: number,
    opts: ClickOptions = {},
  ): Promise<{ x: number; y: number } | undefined> {
    await scrollIntoView(session, backendNodeId)
    try {
      const { x, y } = await getElementCenter(session, backendNodeId)
      await dispatchClick(
        session,
        x,
        y,
        mouseButton(opts.button),
        opts.clickCount ?? 1,
        0,
      )
      return { x, y }
    } catch {
      // No geometry (hidden/zero-size) — fall back to a synthetic DOM click.
      await jsClick(session, backendNodeId)
      return undefined
    }
  }

  async hover(ref: string): Promise<void> {
    const { session, backendNodeId } = await this.observer.resolveRef(ref)
    await this.hoverNode(session, backendNodeId)
  }

  async hoverBackendNode(
    backendNodeId: number,
  ): Promise<{ x: number; y: number }> {
    return this.withPageSessionRetry((session) =>
      this.hoverNode(session, backendNodeId),
    )
  }

  private async hoverNode(
    session: ProtocolApi,
    backendNodeId: number,
  ): Promise<{ x: number; y: number }> {
    await scrollIntoView(session, backendNodeId)
    const { x, y } = await getElementCenter(session, backendNodeId)
    await dispatchHover(session, x, y)
    return { x, y }
  }

  async fill(
    ref: string,
    value: string,
    opts: { clear?: boolean } = {},
  ): Promise<void> {
    const { session, backendNodeId } = await this.observer.resolveRef(ref)
    await this.fillNode(session, backendNodeId, value, opts)
  }

  async fillBackendNode(
    backendNodeId: number,
    value: string,
    opts: { clear?: boolean } = {},
  ): Promise<{ x: number; y: number } | undefined> {
    return this.withPageSessionRetry((session) =>
      this.fillNode(session, backendNodeId, value, opts),
    )
  }

  private async fillNode(
    session: ProtocolApi,
    backendNodeId: number,
    value: string,
    opts: { clear?: boolean } = {},
  ): Promise<{ x: number; y: number } | undefined> {
    await scrollIntoView(session, backendNodeId)

    // A real click is the most reliable way to focus shadow-DOM/custom inputs.
    let coords: { x: number; y: number } | undefined
    try {
      coords = await getElementCenter(session, backendNodeId)
      await dispatchClick(session, coords.x, coords.y, 'left', 1, 0)
    } catch {
      await focusElement(session, backendNodeId)
    }

    const keys = await this.pageSession()
    if (opts.clear !== false) {
      await clearField(keys)
      if (coords && (await getInputValue(session, backendNodeId))) {
        // Still populated — triple-click to select all, then overwrite.
        await dispatchClick(session, coords.x, coords.y, 'left', 3, 0)
      }
    }
    await typeText(keys, value)
    return coords
  }

  async type(text: string): Promise<void> {
    await this.withPageSessionRetry((session) => typeText(session, text))
  }

  async press(key: string): Promise<void> {
    await this.withPageSessionRetry((session) => pressCombo(session, key))
  }

  async selectOption(ref: string, value: string): Promise<string | null> {
    const { session, backendNodeId } = await this.observer.resolveRef(ref)
    return this.selectBackendNodeWithSession(session, backendNodeId, value)
  }

  async selectBackendNode(
    backendNodeId: number,
    value: string,
  ): Promise<string | null> {
    return this.withPageSessionRetry((session) =>
      this.selectBackendNodeWithSession(session, backendNodeId, value),
    )
  }

  private async selectBackendNodeWithSession(
    session: ProtocolApi,
    backendNodeId: number,
    value: string,
  ): Promise<string | null> {
    const selected = await callOnElement(
      session,
      backendNodeId,
      SELECT_OPTION_FN,
      [value],
    )
    return (selected as string | null) ?? null
  }

  async focusBackendNode(backendNodeId: number): Promise<void> {
    await this.withPageSessionRetry(async (session) => {
      await scrollIntoView(session, backendNodeId)
      await focusElement(session, backendNodeId)
    })
  }

  async checkBackendNode(backendNodeId: number): Promise<boolean> {
    const checked = await this.withPageSessionRetry((session) =>
      callOnElement(session, backendNodeId, 'function(){return this.checked}'),
    )
    if (!checked) await this.clickBackendNode(backendNodeId)
    return true
  }

  async uncheckBackendNode(backendNodeId: number): Promise<boolean> {
    const checked = await this.withPageSessionRetry((session) =>
      callOnElement(session, backendNodeId, 'function(){return this.checked}'),
    )
    if (checked) await this.clickBackendNode(backendNodeId)
    return false
  }

  async uploadFile(ref: string, files: string[]): Promise<void>
  async uploadFile(backendNodeId: number, files: string[]): Promise<void>
  async uploadFile(target: string | number, files: string[]): Promise<void> {
    if (typeof target === 'string') {
      const { session, backendNodeId } = await this.observer.resolveRef(target)
      await session.DOM.setFileInputFiles({ files, backendNodeId })
      return
    }

    await this.withPageSessionRetry((session) =>
      session.DOM.setFileInputFiles({ files, backendNodeId: target }),
    )
  }

  async handleDialog(accept: boolean, promptText?: string): Promise<void> {
    await this.withPageSessionRetry((session) =>
      session.Page.handleJavaScriptDialog({
        accept,
        ...(promptText !== undefined && { promptText }),
      }),
    )
  }

  async dragBackendNode(
    sourceBackendNodeId: number,
    target: { element?: number; x?: number; y?: number },
  ): Promise<{
    from: { x: number; y: number }
    to: { x: number; y: number }
  }> {
    return this.withPageSessionRetry(async (session) => {
      await scrollIntoView(session, sourceBackendNodeId)
      const from = await getElementCenter(session, sourceBackendNodeId)

      let to: { x: number; y: number }
      if (target.element !== undefined) {
        to = await getElementCenter(session, target.element)
      } else if (target.x !== undefined && target.y !== undefined) {
        to = { x: target.x, y: target.y }
      } else {
        throw new Error(
          'Provide either target element or both targetX and targetY.',
        )
      }

      await dispatchDrag(session, from, to)
      return { from, to }
    })
  }

  async scroll(
    direction: ScrollDirection,
    amount = 3,
    ref?: string,
  ): Promise<void> {
    const pixels = amount * 120
    const deltaX =
      direction === 'left' ? -pixels : direction === 'right' ? pixels : 0
    const deltaY =
      direction === 'up' ? -pixels : direction === 'down' ? pixels : 0
    if (deltaX === 0 && deltaY === 0) return

    if (ref) {
      const { session, backendNodeId } = await this.observer.resolveRef(ref)
      const { x, y } = await getElementCenter(session, backendNodeId)
      await dispatchScroll(session, x, y, deltaX, deltaY)
      return
    }

    const session = await this.pageSession()
    const metrics = await session.Page.getLayoutMetrics()
    const x = metrics.layoutViewport.clientWidth / 2
    const y = metrics.layoutViewport.clientHeight / 2
    await dispatchScroll(session, x, y, deltaX, deltaY)
  }

  async scrollLegacy(
    direction: string,
    amount: number,
    backendNodeId?: number,
  ): Promise<void> {
    const pixels = amount * 120
    const deltaX =
      direction === 'left' ? -pixels : direction === 'right' ? pixels : 0
    const deltaY =
      direction === 'up' ? -pixels : direction === 'down' ? pixels : 0
    if (deltaX === 0 && deltaY === 0) return

    await this.withPageSessionRetry(async (session) => {
      let x: number
      let y: number
      if (backendNodeId !== undefined) {
        const center = await getElementCenter(session, backendNodeId)
        x = center.x
        y = center.y
      } else {
        const metrics = await session.Page.getLayoutMetrics()
        x = metrics.layoutViewport.clientWidth / 2
        y = metrics.layoutViewport.clientHeight / 2
      }

      const before =
        backendNodeId === undefined
          ? await getWindowScrollPosition(session)
          : undefined
      await dispatchScroll(session, x, y, deltaX, deltaY)
      if (before === undefined) return

      const after = await getWindowScrollPosition(session)
      if (didScrollInExpectedDirection(before, after, deltaX, deltaY)) return

      await fallbackWindowScroll(session, deltaX, deltaY)
    })
  }

  private async pageSession(): Promise<ProtocolApi> {
    return (await this.pages.getSession(this.pageId)).session
  }

  /** Reacquires the page session once after transient CDP reconnect errors. */
  private async withPageSessionRetry<T>(
    action: (session: ProtocolApi) => Promise<T>,
  ): Promise<T> {
    for (let attempt = 0; ; attempt++) {
      try {
        return await action(await this.pageSession())
      } catch (error) {
        if (attempt > 0 || !isRetryableCdpSessionError(error)) throw error
        await this.pages.refresh(this.pageId).catch(() => undefined)
      }
    }
  }
}

function mouseButton(button: ClickOptions['button']): MouseButton {
  if (button === 'middle' || button === 'right') return button
  return 'left'
}

function isRetryableCdpSessionError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return (
    message.includes('CDP not connected') ||
    message.includes('No target with given id') ||
    message.includes('No session with given id') ||
    message.includes('Session with given id not found')
  )
}

async function getWindowScrollPosition(
  session: ProtocolApi,
): Promise<{ x: number; y: number }> {
  const result = await session.Runtime.evaluate({
    expression:
      '({ x: window.scrollX ?? window.pageXOffset ?? 0, y: window.scrollY ?? window.pageYOffset ?? 0 })',
    returnByValue: true,
  })
  const value = (result.result?.value ?? {}) as { x?: number; y?: number }
  return {
    x: typeof value.x === 'number' ? value.x : 0,
    y: typeof value.y === 'number' ? value.y : 0,
  }
}

function didScrollInExpectedDirection(
  before: { x: number; y: number },
  after: { x: number; y: number },
  deltaX: number,
  deltaY: number,
): boolean {
  if (deltaX > 0 && after.x > before.x) return true
  if (deltaX < 0 && after.x < before.x) return true
  if (deltaY > 0 && after.y > before.y) return true
  if (deltaY < 0 && after.y < before.y) return true
  return false
}

async function fallbackWindowScroll(
  session: ProtocolApi,
  deltaX: number,
  deltaY: number,
): Promise<void> {
  await session.Runtime.evaluate({
    expression: `window.scrollBy(${deltaX}, ${deltaY})`,
    returnByValue: true,
  })
}
