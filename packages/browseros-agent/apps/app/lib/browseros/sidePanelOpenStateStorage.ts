import { storage } from '@wxt-dev/storage'

/**
 * Retained only to migrate profiles that enabled the removed window-sharing UI.
 * WXT records version 2 after persisting false, so subsequent startups do not
 * repeat the migration. Native panel initialization is independent of this flag.
 */
export const sidePanelPerWindowStorage = storage.defineItem<boolean>(
  'local:browseros.side_panel.per_window',
  { fallback: false, version: 2, migrations: { 2: () => false } },
)
