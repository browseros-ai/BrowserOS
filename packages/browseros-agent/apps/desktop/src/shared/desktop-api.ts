export type BrowserState = {
  id: number
  url: string
  title: string
  canGoBack: boolean
  canGoForward: boolean
  loading: boolean
  faviconUrl?: string
}

export type BrowserTabState = BrowserState & {
  active: boolean
}

export type AppSurface = 'browser' | 'home' | 'connect-apps' | 'workspace' | 'bookmarks' | 'history' | 'downloads' | 'scheduled' | 'settings'
export type BrowserProfileSummary = { id: string; name: string }
export type DesktopPreferences = {
  showLlmChat: boolean
  showToolbarLabels: boolean
}
export type AssistantLayout = {
  width: number
  mode: 'docked' | 'floating'
}

export type BrowserBounds = {
  x: number
  y: number
  width: number
  height: number
}

export type AgentResult = {
  text: string
  sessionId: string
  raw?: string
}

export type AgentProgress = {
  conversationId: string
  phase: 'started' | 'planning' | 'tool' | 'thinking' | 'verifying' | 'completed' | 'error'
  message: string
  detail?: string
  sequence: number
}

export interface DesktopApi {
  browser: {
    navigate(input: string): Promise<BrowserState>
    goBack(): Promise<BrowserState>
    goForward(): Promise<BrowserState>
    reload(): Promise<BrowserState>
    setBounds(bounds: BrowserBounds): void
    getState(): Promise<BrowserState>
    listTabs(): Promise<BrowserTabState[]>
    createTab(input?: string): Promise<BrowserState>
    activateTab(tabId: number): Promise<BrowserState>
    closeTab(tabId: number): Promise<void>
    bookmarkCurrent(): Promise<void>
    openDevTools(): Promise<void>
    onState(listener: (state: BrowserState) => void): () => void
    onTabs(listener: (tabs: BrowserTabState[]) => void): () => void
  }
  agent: {
    send(goal: string, conversationId: string): Promise<AgentResult>
    onProgress(listener: (progress: AgentProgress) => void): () => void
  }
  workspace: {
    listDatabases(): Promise<Array<{ id: string; name: string; recordCount: number }>>
    listSessions(): Promise<Array<{ id: string; goal: string; status: string }>>
  }
  app: {
    getServerStatus(): Promise<{ ready: boolean; url: string }>
    setAssistantVisible(visible: boolean): Promise<void>
    openSurface(surface: AppSurface): Promise<void>
    showBrowserMenu(): Promise<void>
    showProfileMenu(): Promise<void>
    onFocusAddress(listener: () => void): () => void
    onToggleSidebar(listener: () => void): () => void
    getActiveProfile(): Promise<BrowserProfileSummary>
    getPreferences(): Promise<DesktopPreferences>
    getAssistantLayout(): Promise<AssistantLayout>
    setAssistantWidth(width: number): Promise<{ width: number }>
    setAssistantMode(mode: AssistantLayout['mode']): Promise<{ mode: AssistantLayout['mode'] }>
    onProfile(listener: (profile: BrowserProfileSummary) => void): () => void
    onSurface(listener: (surface: AppSurface) => void): () => void
    onAssistantVisible(listener: (visible: boolean) => void): () => void
    onLog(listener: (line: string) => void): () => void
    onPreferences(listener: (preferences: DesktopPreferences) => void): () => void
    onAssistantLayout(listener: (layout: AssistantLayout) => void): () => void
  }
}
