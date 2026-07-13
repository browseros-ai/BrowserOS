import { contextBridge, ipcRenderer } from 'electron'
import type {
  AgentResult,
  AgentProgress,
  BrowserBounds,
  BrowserState,
  BrowserTabState,
  AppSurface,
  BrowserProfileSummary,
  DesktopPreferences,
  DesktopApi,
} from './shared/desktop-api.js'

const api: DesktopApi = {
  browser: {
    navigate: (input: string): Promise<BrowserState> => ipcRenderer.invoke('browser:navigate', input),
    goBack: (): Promise<BrowserState> => ipcRenderer.invoke('browser:back'),
    goForward: (): Promise<BrowserState> => ipcRenderer.invoke('browser:forward'),
    reload: (): Promise<BrowserState> => ipcRenderer.invoke('browser:reload'),
    setBounds: (bounds: BrowserBounds): void => ipcRenderer.send('browser:set-bounds', bounds),
    getState: (): Promise<BrowserState> => ipcRenderer.invoke('browser:get-state'),
    listTabs: (): Promise<BrowserTabState[]> => ipcRenderer.invoke('browser:list-tabs'),
    createTab: (input?: string): Promise<BrowserState> => ipcRenderer.invoke('browser:create-tab', input),
    activateTab: (tabId: number): Promise<BrowserState> => ipcRenderer.invoke('browser:activate-tab', tabId),
    closeTab: (tabId: number): Promise<void> => ipcRenderer.invoke('browser:close-tab', tabId),
    bookmarkCurrent: (): Promise<void> => ipcRenderer.invoke('browser:bookmark-current'),
    openDevTools: (): Promise<void> => ipcRenderer.invoke('browser:devtools'),
    onState: (listener: (state: BrowserState) => void): (() => void) => {
      const handler = (_event: Electron.IpcRendererEvent, state: unknown) =>
        listener(state as Parameters<typeof listener>[0])
      ipcRenderer.on('browser:state', handler)
      return () => ipcRenderer.removeListener('browser:state', handler)
    },
    onTabs: (listener: (tabs: BrowserTabState[]) => void): (() => void) => {
      const handler = (_event: Electron.IpcRendererEvent, tabs: BrowserTabState[]) => listener(tabs)
      ipcRenderer.on('browser:tabs', handler)
      return () => ipcRenderer.removeListener('browser:tabs', handler)
    },
  },
  agent: {
    send: (goal: string, conversationId: string): Promise<AgentResult> =>
      ipcRenderer.invoke('agent:send', { goal, conversationId }),
    onProgress: (listener: (progress: AgentProgress) => void) => {
      const handler = (_event: Electron.IpcRendererEvent, progress: AgentProgress) => listener(progress)
      ipcRenderer.on('agent:progress', handler)
      return () => ipcRenderer.removeListener('agent:progress', handler)
    },
  },
  workspace: {
    listDatabases: () => ipcRenderer.invoke('workspace:databases'),
    listSessions: () => ipcRenderer.invoke('workspace:sessions'),
  },
  app: {
    getServerStatus: () => ipcRenderer.invoke('app:server-status'),
    setAssistantVisible: (visible: boolean): Promise<void> => ipcRenderer.invoke('agent:assistant-visible', visible),
    openSurface: (surface: AppSurface): Promise<void> => ipcRenderer.invoke('app:open-surface', surface),
    showBrowserMenu: (): Promise<void> => ipcRenderer.invoke('app:show-browser-menu'),
    showProfileMenu: (): Promise<void> => ipcRenderer.invoke('app:show-profile-menu'),
    onFocusAddress: (listener: () => void): (() => void) => {
      const handler = () => listener()
      ipcRenderer.on('app:focus-address', handler)
      return () => ipcRenderer.removeListener('app:focus-address', handler)
    },
    onToggleSidebar: (listener: () => void): (() => void) => {
      const handler = () => listener()
      ipcRenderer.on('app:toggle-sidebar', handler)
      return () => ipcRenderer.removeListener('app:toggle-sidebar', handler)
    },
    getActiveProfile: (): Promise<BrowserProfileSummary> => ipcRenderer.invoke('app:get-active-profile'),
    getPreferences: (): Promise<DesktopPreferences> => ipcRenderer.invoke('app:get-preferences'),
    onProfile: (listener: (profile: BrowserProfileSummary) => void): (() => void) => {
      const handler = (_event: Electron.IpcRendererEvent, profile: BrowserProfileSummary) => listener(profile)
      ipcRenderer.on('app:profile', handler)
      return () => ipcRenderer.removeListener('app:profile', handler)
    },
    onSurface: (listener: (surface: AppSurface) => void): (() => void) => {
      const handler = (_event: Electron.IpcRendererEvent, surface: AppSurface) => listener(surface)
      ipcRenderer.on('app:surface', handler)
      return () => ipcRenderer.removeListener('app:surface', handler)
    },
    onAssistantVisible: (listener: (visible: boolean) => void): (() => void) => {
      const handler = (_event: Electron.IpcRendererEvent, visible: boolean) => listener(visible)
      ipcRenderer.on('app:assistant-visible', handler)
      return () => ipcRenderer.removeListener('app:assistant-visible', handler)
    },
    onLog: (listener: (line: string) => void): (() => void) => {
      const handler = (_event: Electron.IpcRendererEvent, line: string) =>
        listener(line)
      ipcRenderer.on('app:log', handler)
      return () => ipcRenderer.removeListener('app:log', handler)
    },
    onPreferences: (listener: (preferences: DesktopPreferences) => void): (() => void) => {
      const handler = (_event: Electron.IpcRendererEvent, preferences: DesktopPreferences) => listener(preferences)
      ipcRenderer.on('app:preferences', handler)
      return () => ipcRenderer.removeListener('app:preferences', handler)
    },
  },
}

contextBridge.exposeInMainWorld('requestBrowser', api)
