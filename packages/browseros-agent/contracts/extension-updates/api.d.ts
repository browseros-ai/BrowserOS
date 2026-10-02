/** Reusable API in the BrowserOS neo extension worker and extension pages. */
export interface ExtensionUpdateStatus {
  extensionId: string
  runningVersion: string
  pendingVersion: string | null
  detectedAt: number | null
  attemptedVersion: string | null
}
export interface ExtensionUpdateApi {
  restoreContentScripts(): Promise<void>
  getStatus(): Promise<ExtensionUpdateStatus>
  recordUpdate(version: string): Promise<void>
  applyPendingUpdate(expectedVersion: string): Promise<{ scheduled: boolean }>
}
export interface UpdateHost {
  runtime: {
    id: string
    getManifest(): {
      version: string
      content_scripts?: Array<{
        js?: string[]
        matches?: string[]
        all_frames?: boolean
        exclude_matches?: string[]
        include_globs?: string[]
        exclude_globs?: string[]
      }>
    }
    sendMessage(message: object): Promise<{ persisted?: boolean } | undefined>
    reload(): void
    lastError?: { message?: string }
  }
  storage: {
    local: {
      get(key: string): Promise<Record<string, unknown>>
      set(items: Record<string, unknown>): Promise<void>
      remove(key: string): Promise<void>
    }
  }
  tabs?: {
    query(query: object): Promise<Array<{ id?: number }>>
    sendMessage(tabId: number, message: object): Promise<unknown>
  }
  scripting?: {
    executeScript(
      options: { target: { tabId: number; allFrames: boolean } } & (
        | { files: string[]; func?: never }
        | { func: () => unknown; args: []; files?: never }
      ),
    ): Promise<Array<{ result?: unknown }>>
  }
  browserOS?: {
    getPref(name: string, callback: (pref: { value: unknown }) => void): void
  }
}
export default function createExtensionUpdateApi(
  chrome: UpdateHost,
  options?: { now?: () => number; schedule?: (fn: () => void) => void },
): ExtensionUpdateApi
