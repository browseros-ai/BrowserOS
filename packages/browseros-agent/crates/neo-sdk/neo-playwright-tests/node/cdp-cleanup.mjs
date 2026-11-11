/** Cleanup is complete when Chromium has already removed the target or session. */
export function isTargetGone(error) {
  return /(?:^|: )(?:No session with given id|No target with given id(?: found)?|Target closed|Session closed)\.?$/i.test(
    error?.message || '',
  )
}

export async function cleanupCdp(cdp, method, params) {
  try {
    await cdp.send(method, params)
  } catch (error) {
    if (!isTargetGone(error)) throw error
  }
}
