type BrowserOsVersion = [number, number, number, number]

const CLOSED_TAB_GROUPS_MIN_VERSION: BrowserOsVersion = [0, 52, 2, 0]

/** Fails closed when the installed browser version is missing or malformed. */
export function supportsClosedTabGroups(
  nativeBrowserosVersion: string | undefined,
): boolean {
  const version = parseBrowserosVersion(nativeBrowserosVersion)
  if (!version) return false

  for (const index of [0, 1, 2, 3] as const) {
    if (version[index] > CLOSED_TAB_GROUPS_MIN_VERSION[index]) return true
    if (version[index] < CLOSED_TAB_GROUPS_MIN_VERSION[index]) return false
  }
  return true
}

function parseBrowserosVersion(
  rawVersion: string | undefined,
): BrowserOsVersion | undefined {
  if (rawVersion === undefined) return undefined

  const parts = rawVersion.trim().split('.')
  if (parts.length < 2 || parts.length > 4) return undefined

  const values: number[] = []
  for (const part of parts) {
    if (!/^[0-9]+$/.test(part)) return undefined
    const value = Number(part)
    if (!Number.isSafeInteger(value) || value > 0xffff_ffff) return undefined
    values.push(value)
  }

  const [major, minor, build = 0, patch = 0] = values
  if (major === undefined || minor === undefined) return undefined
  return [major, minor, build, patch]
}
