const CHANGELOG_BASE_URL = 'https://docs.browseros.com/changelog'

type ChangelogVersionConfig = {
  showChangelog: true
  anchor?: string
}

/**
 * Whitelist of extension versions that should show changelog.
 * Only versions in this list will trigger changelog notification.
 * Unlisted versions = no changelog shown.
 *
 * @example
 * '0.0.21': { showChangelog: true }  // opens base changelog URL
 * '0.0.25': { showChangelog: true, anchor: 'v0-0-25' }  // opens changelog#v0-0-25
 */
const CHANGELOG_VERSIONS: Record<string, ChangelogVersionConfig> = {
  '0.0.52': { showChangelog: true },
  '0.0.55': { showChangelog: true, anchor: 'v0-37-0' },
}

export function getChangelogUrl(version: string): string {
  const config = CHANGELOG_VERSIONS[version]
  if (!config?.anchor) return CHANGELOG_BASE_URL
  return `${CHANGELOG_BASE_URL}#${config.anchor}`
}

export function shouldShowChangelog(version: string): boolean {
  return CHANGELOG_VERSIONS[version]?.showChangelog === true
}
