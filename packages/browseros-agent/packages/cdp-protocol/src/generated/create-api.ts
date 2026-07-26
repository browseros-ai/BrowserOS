// ── AUTO-GENERATED from CDP protocol. DO NOT EDIT. ──

import type { ProtocolApi } from './protocol-api'

export type RawSend = (
  method: string,
  params?: Record<string, unknown>,
) => Promise<unknown>

export type RawOn = (
  event: string,
  handler: (params: unknown) => void,
) => () => void

function createDomainProxy(domain: string, send: RawSend, on: RawOn): unknown {
  return new Proxy(Object.create(null), {
    get(_, method: string) {
      if (method === 'on') {
        return (event: string, handler: (params: unknown) => void) =>
          on(`${domain}.${event}`, handler)
      }
      return (params?: Record<string, unknown>) =>
        send(`${domain}.${method}`, params)
    },
  })
}

export function createProtocolApi(send: RawSend, on: RawOn): ProtocolApi {
  return {
    Accessibility: createDomainProxy('Accessibility', send, on),
    Animation: createDomainProxy('Animation', send, on),
    Audits: createDomainProxy('Audits', send, on),
    Autofill: createDomainProxy('Autofill', send, on),
    Bookmarks: createDomainProxy('Bookmarks', send, on),
    BackgroundService: createDomainProxy('BackgroundService', send, on),
    BluetoothEmulation: createDomainProxy('BluetoothEmulation', send, on),
    Browser: createDomainProxy('Browser', send, on),
    CSS: createDomainProxy('CSS', send, on),
    CacheStorage: createDomainProxy('CacheStorage', send, on),
    Cast: createDomainProxy('Cast', send, on),
    CrashReportContext: createDomainProxy('CrashReportContext', send, on),
    DOM: createDomainProxy('DOM', send, on),
    DOMDebugger: createDomainProxy('DOMDebugger', send, on),
    DOMSnapshot: createDomainProxy('DOMSnapshot', send, on),
    DOMStorage: createDomainProxy('DOMStorage', send, on),
    DeviceAccess: createDomainProxy('DeviceAccess', send, on),
    DeviceOrientation: createDomainProxy('DeviceOrientation', send, on),
    Emulation: createDomainProxy('Emulation', send, on),
    EventBreakpoints: createDomainProxy('EventBreakpoints', send, on),
    Extensions: createDomainProxy('Extensions', send, on),
    FedCm: createDomainProxy('FedCm', send, on),
    Fetch: createDomainProxy('Fetch', send, on),
    FileSystem: createDomainProxy('FileSystem', send, on),
    HeadlessExperimental: createDomainProxy('HeadlessExperimental', send, on),
    History: createDomainProxy('History', send, on),
    IO: createDomainProxy('IO', send, on),
    IndexedDB: createDomainProxy('IndexedDB', send, on),
    Input: createDomainProxy('Input', send, on),
    Inspector: createDomainProxy('Inspector', send, on),
    LayerTree: createDomainProxy('LayerTree', send, on),
    Log: createDomainProxy('Log', send, on),
    Media: createDomainProxy('Media', send, on),
    Memory: createDomainProxy('Memory', send, on),
    Network: createDomainProxy('Network', send, on),
    Overlay: createDomainProxy('Overlay', send, on),
    PWA: createDomainProxy('PWA', send, on),
    Page: createDomainProxy('Page', send, on),
    Performance: createDomainProxy('Performance', send, on),
    PerformanceTimeline: createDomainProxy('PerformanceTimeline', send, on),
    Preload: createDomainProxy('Preload', send, on),
    Security: createDomainProxy('Security', send, on),
    ServiceWorker: createDomainProxy('ServiceWorker', send, on),
    SmartCardEmulation: createDomainProxy('SmartCardEmulation', send, on),
    Storage: createDomainProxy('Storage', send, on),
    SystemInfo: createDomainProxy('SystemInfo', send, on),
    Target: createDomainProxy('Target', send, on),
    Tethering: createDomainProxy('Tethering', send, on),
    Tracing: createDomainProxy('Tracing', send, on),
    WebAudio: createDomainProxy('WebAudio', send, on),
    WebAuthn: createDomainProxy('WebAuthn', send, on),
    WebMCP: createDomainProxy('WebMCP', send, on),
    Console: createDomainProxy('Console', send, on),
    Debugger: createDomainProxy('Debugger', send, on),
    HeapProfiler: createDomainProxy('HeapProfiler', send, on),
    Profiler: createDomainProxy('Profiler', send, on),
    Runtime: createDomainProxy('Runtime', send, on),
    Schema: createDomainProxy('Schema', send, on),
  } as unknown as ProtocolApi
}
