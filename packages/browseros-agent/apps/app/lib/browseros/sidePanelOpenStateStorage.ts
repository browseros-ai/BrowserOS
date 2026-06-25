import { storage } from '@wxt-dev/storage'

export const sidePanelPerWindowStorage = storage.defineItem<boolean>(
  'local:browseros.side_panel.per_window',
  { fallback: false },
)

export const openWindowSidePanelIdsStorage = storage.defineItem<number[]>(
  'session:browseros.side_panel.open_window_ids',
  { fallback: [] },
)
