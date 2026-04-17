import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'

// TODO(credits-identity): temporary shim — reuses the BrowserOS metrics
// install_id as the credits/referral identifier. Replace with a dedicated
// identity module once we have one.
export async function getBrowserosId(): Promise<string> {
  const adapter = getBrowserOSAdapter()
  const pref = await adapter.getPref(BROWSEROS_PREFS.INSTALL_ID)
  const id = pref.value
  if (typeof id !== 'string' || id.length === 0) {
    throw new Error('browseros.metrics_install_id is not set')
  }
  return id
}
