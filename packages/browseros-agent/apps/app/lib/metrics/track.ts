import { getBrowserOSAdapter } from '@/lib/browseros/adapter'

const versions = {
  extension: null as string | null,
  chromium: null as string | null,
  browseros: null as string | null,
}

const adapter = getBrowserOSAdapter()
adapter
  .getVersion()
  .then((v) => {
    versions.chromium = v
  })
  .catch(() => {})
adapter
  .getBrowserosVersion()
  .then((v) => {
    versions.browseros = v
  })
  .catch(() => {})

/** @public */
export function track(
  eventName: string,
  properties?: Record<string, unknown>,
): void {
  if (!versions.extension) {
    versions.extension = chrome.runtime.getManifest().version
  }

  adapter
    .logMetric(eventName, {
      extension_version: versions.extension,
      ...(versions.chromium && { chromium_version: versions.chromium }),
      ...(versions.browseros && { browseros_version: versions.browseros }),
      ...properties,
    })
    .catch(() => {})
}
