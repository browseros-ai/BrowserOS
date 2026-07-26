const NATIVE_ADDON_DISABLED_MESSAGE =
  'BrowserOS server disables native addon loading in compiled production builds'

interface GuardedProcess extends NodeJS.Process {
  __browserosNativeAddonGuardInstalled?: boolean
}

/** Blocks native addons before Bun can extract bundled `.node` files. */
export function installNativeAddonGuard(): void {
  const guardedProcess = process as GuardedProcess
  if (guardedProcess.__browserosNativeAddonGuardInstalled) return

  const guard: NodeJS.Process['dlopen'] = () => {
    throw new Error(NATIVE_ADDON_DISABLED_MESSAGE)
  }

  process.dlopen = guard
  guardedProcess.__browserosNativeAddonGuardInstalled = true
}
