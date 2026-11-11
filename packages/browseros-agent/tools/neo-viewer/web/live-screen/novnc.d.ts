/**
 * The part of noVNC 1.7's `RFB` the live screen uses (docs/API.md at v1.7.0). The package ships
 * no types, and `@types/novnc__novnc` describes `@novnc/novnc/lib/rfb`, a path 1.7 no longer has.
 */
declare module '@novnc/novnc' {
  export interface RfbEventMap {
    connect: CustomEvent<Record<string, never>>
    disconnect: CustomEvent<{ clean: boolean }>
    securityfailure: CustomEvent<{ status: number; reason?: string }>
    desktopname: CustomEvent<{ name: string }>
  }

  export default class RFB extends EventTarget {
    constructor(
      target: HTMLElement,
      urlOrChannel: string | WebSocket,
      options?: { shared?: boolean; wsProtocols?: string[] },
    )
    viewOnly: boolean
    focusOnClick: boolean
    clipViewport: boolean
    scaleViewport: boolean
    resizeSession: boolean
    showDotCursor: boolean
    background: string
    disconnect(): void
    focus(options?: FocusOptions): void
    addEventListener<K extends keyof RfbEventMap>(
      type: K,
      listener: (event: RfbEventMap[K]) => void,
    ): void
  }
}
