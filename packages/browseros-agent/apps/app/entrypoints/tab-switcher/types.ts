export type LayoutMode = 'strip' | 'grid'

export interface SwitcherState {
  phase: 'idle' | 'open'
  tabs: chrome.tabs.Tab[]
  selectedIndex: number
  thumbnails: Record<number, string>
  searchQuery: string
  layoutMode: LayoutMode
}

export interface SwitcherController {
  advance: (direction: 1 | -1) => void
  select: () => void
  dismiss: () => void
  setSearchQuery: (query: string) => void
  state: SwitcherState
}
