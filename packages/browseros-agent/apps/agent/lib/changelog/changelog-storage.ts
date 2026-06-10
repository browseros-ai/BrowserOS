import { storage } from '@wxt-dev/storage'

const changelogShownStorage = storage.defineItem<string[]>(
  'local:changelogShownVersions',
  { fallback: [] },
)

export async function hasShownChangelog(version: string): Promise<boolean> {
  const shown = await changelogShownStorage.getValue()
  return shown.includes(version)
}

export async function markChangelogShown(version: string): Promise<void> {
  const shown = await changelogShownStorage.getValue()
  if (!shown.includes(version)) {
    await changelogShownStorage.setValue([...shown, version])
  }
}
