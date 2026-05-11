/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * L3 Extension Message Bridge — sends messages to extension service workers
 * via CDP injection (Option C: zero C++ changes).
 *
 * Two-phase discovery:
 *   1. ServiceWorker.enable() -> force-start SW -> getTargets()
 *   2. Attach to SW target -> Runtime.evaluate(chrome.runtime.sendMessage)
 *
 * V1 limitation: only sync onMessage responders are reliable in MV3.
 */

import type { CdpBackend } from './backends/types'

export interface MessageableExtension {
  id: string
  name: string
  type: 'service_worker' | 'background_page'
  hasActiveBackground: boolean
  targetId: string
  url: string
}

/** Derive scope URL from extension service worker script URL */
export function deriveScopeURL(scriptUrl: string): string {
  // chrome-extension://ext-id/path/to/sw.js -> chrome-extension://ext-id/
  const match = scriptUrl.match(/^(chrome-extension:\/\/[^/]+\/)/)
  if (!match) return scriptUrl
  return match[1]
}

/** Extract extension ID from chrome-extension:// URL */
function extractExtensionId(url: string): string | null {
  // Extension IDs are 32-char base16 strings or alphanumeric
  // Support any non-empty ID between chrome-extension:// and /
  const match = url.match(/^chrome-extension:\/\/([a-z0-9_-]+)\//)
  return match?.[1] ?? null
}

/**
 * List extensions that have active (running) service workers
 * discoverable via CDP Target.getTargets().
 */
export async function listMessageableExtensions(
  cdp: CdpBackend,
): Promise<MessageableExtension[]> {
  // Enable SW events to ensure discovery works
  await cdp.ServiceWorker.enable()

  const targets = await cdp.getTargets()

  const results: MessageableExtension[] = []

  for (const target of targets) {
    if (!target.url.startsWith('chrome-extension://')) continue
    if (target.type !== 'service_worker') continue

    const extId = extractExtensionId(target.url)
    if (!extId) continue

    results.push({
      id: extId,
      name: target.title || extId,
      type: 'service_worker',
      hasActiveBackground: true,
      targetId: target.id,
      url: target.url,
    })
  }

  return results
}

/**
 * Send a message to an extension's onMessage listener via CDP injection.
 *
 * How it works:
 *   1. Discover the extension's service worker target
 *   2. Attach to it via CDP
 *   3. Evaluate chrome.runtime.sendMessage(msg, callback) in that context
 *   4. Return the response
 *
 * @param cdp - CDP backend
 * @param extensionId - Target extension ID
 * @param message - Message payload (must be JSON-serializable)
 * @param timeoutMs - Timeout in ms (default 10000)
 * @returns Response from extension's onMessage listener, or null if no response
 */
export async function sendExtensionMessage(
  cdp: CdpBackend,
  extensionId: string,
  message: unknown,
  timeoutMs = 10000,
): Promise<unknown> {
  // Phase 1: Enable SW and force-start the extension's service worker
  await cdp.ServiceWorker.enable()

  const scopeURL = `chrome-extension://${extensionId}/`

  // Try to start the service worker (may already be running)
  try {
    await (cdp.ServiceWorker as any).startWorker({ scopeURL })
  } catch {
    // SW may already be running — that's fine
  }

  // Phase 2: Discover the SW target
  const targets = await cdp.getTargets()
  const swTarget = targets.find(
    (t) =>
      t.type === 'service_worker' &&
      t.url.startsWith(`chrome-extension://${extensionId}/`),
  )

  if (!swTarget) {
    throw new Error(
      `No discoverable target for extension ${extensionId}. The extension may not have a service worker or it may be suspended.`,
    )
  }

  // Phase 3: Attach and inject message
  const MAX_RETRIES = 2
  let lastError: Error | null = null

  for (let attempt = 0; attempt < MAX_RETRIES; attempt++) {
    try {
      const { sessionId } = await cdp.Target.attachToTarget({
        targetId: swTarget.id,
        flatten: true,
      })

      const session = cdp.session(sessionId)

      // Enable Runtime to evaluate expressions
      await session.Runtime.enable()

      // Inject chrome.runtime.sendMessage into the extension's SW context
      const serializedMsg = JSON.stringify(message)
      const evaluateExpr = `
        new Promise((resolve) => {
          try {
            chrome.runtime.sendMessage(${serializedMsg}, (response) => {
              resolve(JSON.stringify(response ?? null));
            });
          } catch (e) {
            resolve(JSON.stringify({ __error: e.message }));
          }
        })
      `

      const timeoutPromise = new Promise<never>((_, reject) =>
        setTimeout(
          () => reject(new Error(`Extension message timeout (${timeoutMs}ms)`)),
          timeoutMs,
        ),
      )

      const evalResult = await Promise.race([
        session.Runtime.evaluate({
          expression: evaluateExpr,
          awaitPromise: true,
          returnByValue: true,
        }),
        timeoutPromise,
      ])

      // Detach after evaluation
      try {
        await cdp.Target.detachFromTarget({ sessionId })
      } catch {
        // Ignore detach errors
      }

      const value = (evalResult as any)?.result?.value
      if (value === undefined || value === 'undefined') {
        return null
      }

      try {
        const parsed = JSON.parse(value)
        if (parsed?.__error) {
          throw new Error(`Extension error: ${parsed.__error}`)
        }
        return parsed
      } catch {
        // Not JSON — return raw value
        return value
      }
    } catch (err) {
      lastError = err instanceof Error ? err : new Error(String(err))
      // Retry on attach failure (SW may have suspended between discover and attach)
      if (
        lastError.message.includes('Target closed') ||
        lastError.message.includes('session')
      ) {
        // Force restart SW and retry
        try {
          await (cdp.ServiceWorker as any).startWorker({ scopeURL })
        } catch {
          // Ignore
        }
        continue
      }
      throw lastError
    }
  }

  throw lastError!
}
