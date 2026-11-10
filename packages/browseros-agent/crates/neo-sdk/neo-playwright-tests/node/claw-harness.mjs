/** Private CDP for test environment setup and reset; never passed to upstream tests. */
import { Cdp } from '../js/browser.mjs'
import { installTestBuiltins } from '../js/test-builtins.mjs'

const activeTests = new WeakMap()

export function holdHarness(cdp) {
  const tests = activeTests.get(cdp) || new Set()
  activeTests.set(cdp, tests)
  const { promise, resolve } = Promise.withResolvers()
  tests.add(promise)
  return () => {
    tests.delete(promise)
    resolve()
  }
}

export async function closeHarness(cdp) {
  // An unhandled SDK rejection can overlap Playwright's test and worker teardown.
  await Promise.all(activeTests.get(cdp) || [])
  await cdp.close()
}

export async function openHarness(port) {
  const response = await fetch(`http://127.0.0.1:${port}/json/version`)
  if (!response.ok)
    throw new Error(`harness-error: CDP discovery returned ${response.status}`)
  return await Cdp.connect((await response.json()).webSocketDebuggerUrl)
}

export async function pageTargets(cdp) {
  const { targetInfos } = await cdp.send('Target.getTargets')
  return targetInfos
    .filter((target) => target.type === 'page')
    .map((target) => target.targetId)
}

export async function installFixtureBuiltins(cdp, before) {
  const added = (await pageTargets(cdp)).filter((id) => !before.includes(id))
  if (added.length !== 1)
    throw new Error(
      `harness-error: fixture page target diff has ${added.length} pages`,
    )
  const { sessionId } = await cdp.send('Target.attachToTarget', {
    targetId: added[0],
    flatten: true,
  })
  try {
    await cdp.send('Page.enable', {}, sessionId)
    await installTestBuiltins(cdp, sessionId)
    // Chromium removes this session's new-document scripts when it detaches.
    return async () => {
      if ((await pageTargets(cdp)).includes(added[0]))
        await cdp.send('Target.detachFromTarget', { sessionId })
    }
  } catch (error) {
    await cdp.send('Target.detachFromTarget', { sessionId })
    throw error
  }
}

export function serverOrigins(port, loopback) {
  const hosts = new Set(
    ['localhost', '127.0.0.1', '[::1]', loopback].filter(Boolean),
  )
  return [...hosts].flatMap((host) => [
    `http://${host}:${port}`,
    `https://${host}:${port + 1}`,
  ])
}

export async function resetClawTest({ context, initialPages, cdp, origins }) {
  const errors = []
  // Closing the test's tabs also destroys their sessionStorage. The launch tab is retained.
  for (const page of context.pages()) {
    if (initialPages.has(page)) continue
    try {
      await page.close()
    } catch (error) {
      errors.push(error)
    }
  }
  try {
    context.removeAllListeners()
  } catch (error) {
    errors.push(error)
  }
  try {
    await clearProfileData(cdp, origins)
  } catch (error) {
    errors.push(error)
  }
  if (errors.length)
    throw new AggregateError(
      errors,
      `harness-error: Claw test reset failed: ${errors.map((error) => error.message).join('; ')}`,
    )
}

async function clearProfileData(cdp, origins) {
  let [targetId] = await pageTargets(cdp)
  const temporary = !targetId
  if (temporary) {
    const { tab } = await cdp.send('Browser.createTab', {
      url: 'about:blank',
      background: true,
    })
    targetId = tab.targetId
  }
  let sessionId
  try {
    // Storage.clearDataForOrigin needs a target's storage partition even on a browser socket.
    ;({ sessionId } = await cdp.send('Target.attachToTarget', {
      targetId,
      flatten: true,
    }))
    await cdp.send('Storage.clearCookies')
    await cdp.send('Network.clearBrowserCache', {}, sessionId)
    for (const origin of origins)
      await cdp.send(
        'Storage.clearDataForOrigin',
        { origin, storageTypes: 'all' },
        sessionId,
      )
    const { result } = await cdp.send(
      'Runtime.evaluate',
      { expression: 'location.origin', returnByValue: true },
      sessionId,
    )
    if (origins.includes(result.value))
      await cdp.send(
        'DOMStorage.clear',
        { storageId: { securityOrigin: result.value, isLocalStorage: false } },
        sessionId,
      )
  } finally {
    try {
      if (sessionId) await cdp.send('Target.detachFromTarget', { sessionId })
    } finally {
      if (temporary) await cdp.send('Target.closeTarget', { targetId })
    }
  }
}
