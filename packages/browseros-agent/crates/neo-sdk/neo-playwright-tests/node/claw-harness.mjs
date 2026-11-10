/** Private CDP for test environment setup and reset; never passed to upstream tests. */
import { Cdp } from '../js/browser.mjs'
import { installTestBuiltins } from '../js/test-builtins.mjs'

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
    await installTestBuiltins(cdp, sessionId)
  } finally {
    await cdp.send('Target.detachFromTarget', { sessionId })
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
    await cdp.send('Storage.clearCookies')
    for (const origin of origins)
      await cdp.send('Storage.clearDataForOrigin', {
        origin,
        storageTypes: 'all',
      })
  } catch (error) {
    errors.push(error)
  }
  if (errors.length)
    throw new AggregateError(
      errors,
      `harness-error: Claw test reset failed: ${errors.map((error) => error.message).join('; ')}`,
    )
}
