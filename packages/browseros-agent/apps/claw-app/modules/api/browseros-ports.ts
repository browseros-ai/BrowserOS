/// <reference path="./chrome-browser-os.d.ts" />

import { CLAW_API_PORT_DEFAULT } from '@browseros/claw-server/shared/port'
import {
  API_URL_STORAGE_KEY,
  type ApiBaseUrlSources,
  normalizeLoopbackApiRootUrl,
  resolveApiBaseUrlFromSources,
} from './client.helpers'

const BROWSEROS_SERVER_PORT_PREF = 'browseros.server.server_port'
const BROWSEROS_PROXY_PORT_PREF = 'browseros.server.proxy_port'

export function fallbackClawApiBaseUrl(): string {
  return `http://127.0.0.1:${CLAW_API_PORT_DEFAULT}`
}

/** Builds trusted fallback sources from the current extension window. */
export function apiBaseUrlSourcesFromWindow(
  fallback = fallbackClawApiBaseUrl(),
): ApiBaseUrlSources {
  if (typeof window === 'undefined') {
    return {
      query: null,
      stored: null,
      launcher: null,
      fallback,
    }
  }

  const query = new URLSearchParams(window.location.search).get('apiUrl')
  const queryBaseUrl = normalizeLoopbackApiRootUrl(query)
  if (queryBaseUrl) {
    try {
      window.sessionStorage.setItem(API_URL_STORAGE_KEY, queryBaseUrl)
    } catch {}
  }

  let stored: string | null = null
  try {
    stored = window.sessionStorage.getItem(API_URL_STORAGE_KEY)
  } catch {}

  return {
    query,
    stored,
    launcher: import.meta.env.VITE_BROWSEROS_CLAW_API_URL,
    fallback,
  }
}

/** Resolves the BrowserOS-managed sidecar server base URL. */
export async function resolveBrowserOSServerBaseUrl(
  sources = apiBaseUrlSourcesFromWindow(),
): Promise<string> {
  const port = await readBrowserOSPort(BROWSEROS_SERVER_PORT_PREF)
  return port ? loopbackBaseUrl(port) : resolveApiBaseUrlFromSources(sources)
}

/** Resolves the BrowserOS-managed MCP proxy base URL. */
export async function resolveBrowserOSMcpBaseUrl(
  sources = apiBaseUrlSourcesFromWindow(),
): Promise<string> {
  const port = await readBrowserOSPort(BROWSEROS_PROXY_PORT_PREF)
  return port ? loopbackBaseUrl(port) : resolveApiBaseUrlFromSources(sources)
}

async function readBrowserOSPort(prefName: string): Promise<number | null> {
  if (
    typeof chrome === 'undefined' ||
    typeof chrome.browserOS?.getPref !== 'function'
  ) {
    return null
  }

  try {
    const pref = await new Promise<chrome.browserOS.PrefObject>(
      (resolve, reject) => {
        chrome.browserOS.getPref(prefName, (value) => {
          const message = chrome.runtime?.lastError?.message
          if (message) {
            reject(new Error(message))
            return
          }
          resolve(value)
        })
      },
    )
    return validPort(pref.value)
  } catch {
    return null
  }
}

function validPort(value: unknown): number | null {
  if (typeof value !== 'number') return null
  return Number.isInteger(value) && value > 0 && value <= 65535 ? value : null
}

function loopbackBaseUrl(port: number): string {
  return `http://127.0.0.1:${port}`
}
