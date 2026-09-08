import {
  createDiagnosticsCollector,
  type DiagnosticsResult,
  within,
} from './collector'
import {
  DIAGNOSTICS_MAX_AGE_MS,
  DIAGNOSTICS_REQUEST,
  DIAGNOSTICS_TIMEOUT_MS,
  type DiagnosticsSnapshot,
  parseDiagnostics,
  REPORTER_EXTENSION_ID,
  REPORTER_VERSION_REQUEST,
} from './contract'

const CACHE_KEY = 'browseros.diagnostics.v1'
const REFRESH_ALARM = 'browseros.diagnostics.refresh'
type Product = 'browseros' | 'browseros-neo'
interface NativeVersions {
  getVersionNumber?: (callback: (value: string) => void) => void
  getBrowserosVersionNumber?: (callback: (value: string) => void) => void
}
interface ServerDetails {
  version: string | null
  os: string | null
  osVersion: string | null
}
const emptyServer: ServerDetails = { version: null, os: null, osVersion: null }
const valueText = (value: unknown): string | null =>
  typeof value === 'string' ? value.slice(0, 200) : null

async function nativeVersion(
  api: NativeVersions | undefined,
  name: keyof NativeVersions,
): Promise<string | null> {
  if (!api?.[name]) return null
  return new Promise((resolve, reject) => {
    api[name]?.((value) => {
      const error = chrome.runtime.lastError
      if (error) reject(new Error(error.message))
      else resolve(valueText(value))
    })
  })
}

async function readServer(
  resolveServerUrl: () => Promise<string>,
): Promise<ServerDetails> {
  const baseUrl = await resolveServerUrl()
  const response = await fetch(`${baseUrl}/system/diagnostics`, {
    signal: AbortSignal.timeout(1800),
  })
  if (!response.ok) throw new Error('Server diagnostics unavailable')
  const data = await response.json()
  return {
    version: valueText(data.version),
    os: valueText(data.os),
    osVersion: valueText(data.osVersion),
  }
}

/** Native reads survive a stopped agent server. Only selected metadata enters the snapshot. */
export async function collectBrowserDiagnostics(
  product: Product,
  resolveServerUrl: () => Promise<string>,
): Promise<DiagnosticsSnapshot> {
  const api = (chrome as unknown as { browserOS?: NativeVersions }).browserOS
  const safe = <T>(task: () => Promise<T>) =>
    within(task, 2000).catch(() => null)
  const [browseros, chromium, platform, cpu, memory, server, reporter] =
    await Promise.all([
      safe(() => nativeVersion(api, 'getBrowserosVersionNumber')),
      safe(() => nativeVersion(api, 'getVersionNumber')),
      safe(() => chrome.runtime.getPlatformInfo()),
      safe(() => chrome.system.cpu.getInfo()),
      safe(() => chrome.system.memory.getInfo()),
      safe(() => readServer(resolveServerUrl)),
      safe(() =>
        chrome.runtime.sendMessage(REPORTER_EXTENSION_ID, {
          type: REPORTER_VERSION_REQUEST,
        }),
      ),
    ])
  const osNames: Record<string, string> = {
    mac: 'macOS',
    win: 'Windows',
    linux: 'Linux',
    cros: 'ChromeOS',
    android: 'Android',
    openbsd: 'OpenBSD',
  }
  const details = server ?? emptyServer
  return {
    schemaVersion: 1,
    product,
    collectedAt: new Date().toISOString(),
    versions: {
      browseros,
      chromium,
      server: details.version,
      appExtension: chrome.runtime.getManifest().version,
      reporterExtension: valueText(reporter?.version),
    },
    system: {
      os:
        details.os ?? (platform ? (osNames[platform.os] ?? platform.os) : null),
      osVersion: details.osVersion,
      architecture: platform?.arch ?? null,
      processor: valueText(cpu?.modelName),
      logicalCores: cpu?.numOfProcessors ?? null,
    },
    memory: {
      totalBytes: memory?.capacity ?? null,
      availableBytes: memory?.availableCapacity ?? null,
    },
  }
}

/** Separate own extension pages from content scripts and external websites. */
export function canReadDiagnostics(
  sender: { id?: string; url?: string },
  ownId: string,
  external: boolean,
): boolean {
  if (external) return sender.id === REPORTER_EXTENSION_ID
  return (
    sender.id === ownId &&
    !!sender.url?.startsWith(`chrome-extension://${ownId}/`)
  )
}

/**
 * Register synchronously at worker startup, before any server connection awaits.
 * Chrome wakes this worker on a message; cache persistence handles later suspension.
 */
export function registerDiagnostics(
  product: Product,
  resolveServerUrl: () => Promise<string>,
): void {
  const collector = createDiagnosticsCollector({
    read: () => collectBrowserDiagnostics(product, resolveServerUrl),
    load: async () => (await chrome.storage.local.get(CACHE_KEY))[CACHE_KEY],
    save: async (snapshot) => {
      await chrome.storage.local.set({ [CACHE_KEY]: snapshot })
    },
  })
  const listener =
    (external: boolean) =>
    (
      request: unknown,
      sender: chrome.runtime.MessageSender,
      respond: (result: unknown) => void,
    ) => {
      if (
        !request ||
        typeof request !== 'object' ||
        !('type' in request) ||
        request.type !== DIAGNOSTICS_REQUEST
      )
        return false
      if (!canReadDiagnostics(sender, chrome.runtime.id, external)) return false
      const age =
        'maxAgeMs' in request &&
        typeof request.maxAgeMs === 'number' &&
        Number.isFinite(request.maxAgeMs)
          ? Math.min(DIAGNOSTICS_MAX_AGE_MS, Math.max(0, request.maxAgeMs))
          : DIAGNOSTICS_MAX_AGE_MS
      void collector
        .get(age)
        .then(respond)
        .catch(() => respond({ snapshot: null, cached: false }))
      // Shipped Chromium versions require this for asynchronous sendResponse.
      return true
    }
  chrome.runtime.onMessage.addListener(listener(false))
  chrome.runtime.onMessageExternal.addListener(listener(true))
  const refresh = () => {
    void collector.get(0).catch(() => {})
  }
  const start = () => {
    void chrome.alarms
      .create(REFRESH_ALARM, { periodInMinutes: 360 })
      .catch(() => {})
    refresh()
  }
  chrome.runtime.onInstalled.addListener(start)
  chrome.runtime.onStartup.addListener(start)
  chrome.alarms.onAlarm.addListener((alarm) => {
    if (alarm.name === REFRESH_ALARM) refresh()
  })
}

/** UI requests share the worker's collector; they never infer system facts themselves. */
export async function requestDiagnostics(
  maxAgeMs = 0,
): Promise<DiagnosticsResult> {
  try {
    const response = await within(
      () =>
        chrome.runtime.sendMessage({
          type: DIAGNOSTICS_REQUEST,
          maxAgeMs,
          // The app's @webext-core listeners share this runtime channel and
          // validate the envelope before checking the type. Without a timestamp
          // they reject our request before the diagnostics handler can answer.
          timestamp: Date.now(),
        }),
      DIAGNOSTICS_TIMEOUT_MS,
    )
    const snapshot = parseDiagnostics(response?.snapshot)
    if (snapshot) return { snapshot, cached: response.cached === true }
  } catch {}
  const saved = await within(
    () => chrome.storage.local.get(CACHE_KEY),
    200,
  ).catch(() => ({}))
  const snapshot = parseDiagnostics(
    (saved as Record<string, unknown>)[CACHE_KEY],
  )
  if (!snapshot)
    throw new Error('Diagnostics could not be collected. Try refreshing.')
  return { snapshot, cached: true }
}
