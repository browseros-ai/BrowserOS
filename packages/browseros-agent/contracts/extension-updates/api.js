/**
 * Owns staged-update state in the extension's profile. Bundled by claw-app and
 * evaluated by the sidecar for older extensions, so both use identical guards.
 * Keep this factory self-contained: CDP supplies Chrome APIs, not module imports.
 */
export default function createExtensionUpdateApi(chrome, options = {}) {
  const key = 'browseros.extensionUpdate'
  const now = options.now ?? Date.now
  const schedule = options.schedule ?? ((fn) => setTimeout(fn, 150))
  let queue = Promise.resolve()
  let restoration = null
  const restored = new Set()

  // Storage read/modify/write and reload decisions share one worker-local queue.
  // Persisting the attempt before reload also protects across worker/server restarts.
  function serial(operation) {
    const result = queue.then(operation)
    queue = result.then(
      () => {},
      () => {},
    )
    return result
  }
  function versionParts(value) {
    if (typeof value !== 'string' || !/^\d+(\.\d+){0,3}$/.test(value))
      return null
    const parts = value.split('.').map(Number)
    if (parts.some((part) => !Number.isInteger(part) || part > 65535))
      return null
    while (parts.length < 4) parts.push(0)
    return parts
  }
  function newer(candidate, current) {
    const a = versionParts(candidate),
      b = versionParts(current)
    if (!a || !b) return false
    for (let i = 0; i < 4; i++) {
      if (a[i] !== b[i]) return a[i] > b[i]
    }
    return false
  }
  async function withTimeout(promise, milliseconds) {
    let timer
    try {
      return await Promise.race([
        promise,
        new Promise((_, reject) => {
          timer = setTimeout(
            () => reject(new Error('Extension update operation timed out')),
            milliseconds,
          )
        }),
      ])
    } finally {
      clearTimeout(timer)
    }
  }
  async function remember(version) {
    const previous = (await chrome.storage.local.get(key))[key]
    const record =
      previous?.version === version &&
      Number.isFinite(previous.detectedAt) &&
      previous.detectedAt >= 0 &&
      previous.detectedAt <= now()
        ? previous
        : { version, detectedAt: now(), attemptedVersion: null }
    await chrome.storage.local.set({ [key]: record })
    return record
  }
  function restoreContentScripts() {
    if (!chrome.scripting || !chrome.tabs) return Promise.resolve()
    // onInstalled and reconciliation await the same recovery. A failed startup
    // keeps the durable marker so the next check can retry without reloading.
    restoration ??= (async () => {
      for (const script of chrome.runtime.getManifest().content_scripts ?? []) {
        if (
          !script.js?.length ||
          !script.matches?.includes('<all_urls>') ||
          script.exclude_matches?.length ||
          script.include_globs?.length ||
          script.exclude_globs?.length
        )
          continue
        const tabs = await withTimeout(
          chrome.tabs.query({
            url: ['http://*/*', 'https://*/*', 'file:///*'],
          }),
          2000,
        )
        const results = await Promise.allSettled(
          tabs
            .filter((tab) => tab.id != null && !tab.discarded)
            .map(async (tab) => {
              const identity = `${tab.id}:${script.js.join(',')}`
              if (restored.has(identity)) return
              const target = {
                tabId: tab.id,
                allFrames: script.all_frames ?? false,
              }
              // Legacy recorders used a window flag that survives extension reload.
              // They were stopped before reload; release only our own duplicate guard.
              await withTimeout(
                chrome.scripting.executeScript({
                  target,
                  func: () => {
                    delete window.__browserosClawReplayInstalled
                  },
                  args: [],
                }),
                2000,
              )
              await withTimeout(
                chrome.scripting.executeScript({ target, files: script.js }),
                2000,
              )
              restored.add(identity)
            }),
        )
        for (const result of results) {
          if (result.status !== 'rejected') continue
          // Chrome forbids protected documents and a tab/frame can close while
          // we inject. Other failures retain the marker for a later retry.
          if (
            /Cannot access|Missing host permission|extensions gallery|No tab with id|No frame with id|Frame with ID .*removed/i.test(
              String(result.reason),
            )
          )
            continue
          throw result.reason
        }
      }
    })().catch((error) => {
      restoration = null
      throw error
    })
    return restoration
  }
  async function status() {
    const runningVersion = chrome.runtime.getManifest().version
    // Read the in-memory native preference, in this worker's own profile. Never
    // write it or use a requestUpdateCheck response as proof of a staged CRX.
    const settings = await withTimeout(
      new Promise((resolve, reject) => {
        if (!chrome.browserOS?.getPref)
          return reject(new Error('Native update status unavailable'))
        chrome.browserOS.getPref('extensions.settings', (pref) => {
          const error = chrome.runtime.lastError?.message
          if (error) reject(new Error(error))
          else if (!pref?.value || typeof pref.value !== 'object')
            reject(new Error('Invalid native update status'))
          else resolve(pref.value)
        })
      }),
      5000,
    )
    const pending = settings[chrome.runtime.id]?.idle_install_info
    const version = pending?.manifest?.version
    const ready =
      pending?.delay_install_reason === 2 && newer(version, runningVersion)
    const record = ready ? await remember(version) : null
    if (!ready) {
      const previous = (await chrome.storage.local.get(key))[key]
      // A newer extension may be an older release without our onInstalled hook.
      // Recover its content scripts too, including after a sidecar restart.
      if (previous?.version && !newer(previous.version, runningVersion))
        await restoreContentScripts()
      await chrome.storage.local.remove(key)
    }
    return {
      extensionId: chrome.runtime.id,
      runningVersion,
      pendingVersion: record?.version ?? null,
      detectedAt: record?.detectedAt ?? null,
      attemptedVersion: record?.attemptedVersion ?? null,
    }
  }
  return {
    restoreContentScripts,
    getStatus: () => serial(status),
    recordUpdate: (version) =>
      serial(async () => {
        if (newer(version, chrome.runtime.getManifest().version))
          await remember(version)
      }),
    applyPendingUpdate: (expectedVersion) =>
      serial(async () => {
        const current = await status()
        if (
          !current.pendingVersion ||
          current.pendingVersion !== expectedVersion ||
          current.attemptedVersion === expectedVersion
        )
          return { scheduled: false }
        // Stop this extension's old recorders before invalidating their contexts.
        // This message already exists in legacy versions, so compatibility reloads
        // do not leave rrweb observers running after the old worker disappears.
        if (chrome.tabs) {
          try {
            await withTimeout(
              (async () => {
                const tabs = await chrome.tabs.query({})
                const stops = await Promise.allSettled(
                  tabs
                    .filter((tab) => tab.id != null)
                    .map(async (tab) => {
                      const response = await chrome.tabs.sendMessage(tab.id, {
                        type: 'recorder-stop',
                      })
                      if (response?.persisted === true) return
                      if (response?.persisted === false)
                        throw new Error('Recorder flush was not persisted')
                      if (!chrome.scripting)
                        throw new Error('Legacy recorder drain is unavailable')
                      // Legacy stop handlers do not reply. After their stop round
                      // trip, enqueue an empty batch from the same document. The
                      // existing relay serializes it behind previous batches and
                      // acknowledges only after outbox persistence. No replay event
                      // is added; a missing acknowledgment prevents reload.
                      const barrier = await chrome.scripting.executeScript({
                        target: { tabId: tab.id, allFrames: false },
                        func: async function flushLegacyRecorder() {
                          const result = await chrome.runtime.sendMessage({
                            type: 'recorder-events',
                            ndjson: '',
                            hasGap: false,
                          })
                          return result?.persisted === true
                        },
                        args: [],
                      })
                      if (
                        !barrier?.length ||
                        barrier.some((entry) => entry.result !== true)
                      )
                        throw new Error(
                          'Legacy recorder flush was not persisted',
                        )
                    }),
                )
                for (const stop of stops) {
                  if (stop.status !== 'rejected') continue
                  // A tab that has no recorder, or has closed, has no live buffer.
                  if (
                    /Receiving end does not exist|No tab with id/i.test(
                      String(stop.reason),
                    )
                  )
                    continue
                  throw stop.reason
                }
              })(),
              2000,
            )
          } catch (error) {
            // No reload was issued. Resume stopped recorders before reporting a
            // failed preparation; a later check may safely try preparation again.
            restoration = null
            restored.clear()
            await restoreContentScripts()
            throw error
          }
        }
        await chrome.storage.local.set({
          [key]: {
            version: expectedVersion,
            detectedAt: current.detectedAt,
            attemptedVersion: expectedVersion,
          },
        })
        // Acknowledge through CDP/runtime messaging before destroying this context.
        schedule(() => chrome.runtime.reload())
        return { scheduled: true }
      }),
  }
}
