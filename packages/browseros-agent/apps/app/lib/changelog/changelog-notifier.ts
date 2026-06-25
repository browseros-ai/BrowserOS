import { getChangelogUrl, shouldShowChangelog } from './changelog-config'
import { hasShownChangelog, markChangelogShown } from './changelog-storage'

const CHANGELOG_DELAY_MS = 5000

function getExtensionVersion(): string {
  return chrome.runtime.getManifest().version
}

export async function checkAndShowChangelog(): Promise<void> {
  const version = getExtensionVersion()

  if (!shouldShowChangelog(version)) {
    return
  }

  if (await hasShownChangelog(version)) {
    return
  }

  setTimeout(async () => {
    const url = getChangelogUrl(version)
    await chrome.tabs.create({ url })
    await markChangelogShown(version)
  }, CHANGELOG_DELAY_MS)
}
