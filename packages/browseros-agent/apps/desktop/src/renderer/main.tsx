import {
  ArrowLeft,
  ArrowRight,
  BookOpen,
  CalendarClock,
  Clock3,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Database,
  Download,
  Globe2,
  PanelRight,
  LoaderCircle,
  LockKeyhole,
  Menu,
  MoreHorizontal,
  Plus,
  PlugZap,
  RefreshCw,
  Search,
  Settings2,
  Sparkles,
  Star,
  SquarePen,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import type { AgentProgress, AppSurface, BrowserProfileSummary, BrowserState, BrowserTabState, DesktopPreferences } from '../shared/desktop-api'
import './styles.css'

type ChatMessage = {
  id: string
  role: 'user' | 'assistant'
  text: string
}

const newConversationId = () => crypto.randomUUID()

function Shell() {
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [assistantOpen, setAssistantOpen] = useState(true)
  const [activeView, setActiveView] = useState<'agent' | 'workspace'>('agent')
  const [surface, setSurface] = useState<AppSurface>('browser')
  const [address, setAddress] = useState('https://www.google.com')
  const [browser, setBrowser] = useState<BrowserState>({
    id: 0,
    url: 'about:blank',
    title: 'New tab',
    canGoBack: false,
    canGoForward: false,
    loading: false,
  })
  const [tabs, setTabs] = useState<BrowserTabState[]>([])
  const [profile, setProfile] = useState<BrowserProfileSummary>({ id: 'default', name: 'Personal' })
  const [preferences, setPreferences] = useState<DesktopPreferences>({ showLlmChat: true, showToolbarLabels: true })
  const [serverReady, setServerReady] = useState(false)
  const [agentActivity, setAgentActivity] = useState<AgentProgress | null>(null)
  const [goal, setGoal] = useState('')
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [busy, setBusy] = useState(false)
  const [databases, setDatabases] = useState<
    Array<{ id: string; name: string; recordCount: number }>
  >([])
  const [sessions, setSessions] = useState<
    Array<{ id: string; goal: string; status: string }>
  >([])
  const conversationIdRef = useRef(newConversationId())
  const shellRef = useRef<HTMLDivElement>(null)
  const addressInputRef = useRef<HTMLInputElement>(null)

  const displayUrl = useMemo(() => browser.url || address, [browser.url, address])

  useEffect(() => {
    const stopFocus = window.requestBrowser.app.onFocusAddress(() => {
      void openSurface('browser')
      window.setTimeout(() => { addressInputRef.current?.focus(); addressInputRef.current?.select() }, 0)
    })
    const stopSidebar = window.requestBrowser.app.onToggleSidebar(() => setSidebarCollapsed((value) => !value))
    return () => { stopFocus(); stopSidebar() }
  }, [])

  useEffect(() => {
    const unsubscribe = window.requestBrowser.browser.onState((state) => {
      setBrowser(state)
      if (state.url && state.url !== 'about:blank') setAddress(state.url)
    })
    const unsubscribeTabs = window.requestBrowser.browser.onTabs(setTabs)
    window.requestBrowser.browser.getState().then(setBrowser).catch(() => {})
    window.requestBrowser.browser.listTabs().then(setTabs).catch(() => {})
    window.requestBrowser.app.getActiveProfile().then(setProfile).catch(() => {})
    const unsubscribeProfile = window.requestBrowser.app.onProfile(setProfile)
    const unsubscribeSurface = window.requestBrowser.app.onSurface(setSurface)
    const unsubscribeAssistant = window.requestBrowser.app.onAssistantVisible(setAssistantOpen)
    const unsubscribePreferences = window.requestBrowser.app.onPreferences(setPreferences)
    const unsubscribeAgentProgress = window.requestBrowser.agent.onProgress(setAgentActivity)
    let cancelled = false
    const checkServer = () => {
      window.requestBrowser.app.getServerStatus().then((status) => {
        if (!cancelled) setServerReady(status.ready)
      }).catch(() => {})
    }
    checkServer()
    const interval = window.setInterval(checkServer, 750)
    return () => {
      cancelled = true
      window.clearInterval(interval)
      unsubscribe()
      unsubscribeTabs()
      unsubscribeProfile()
      unsubscribeSurface()
      unsubscribeAssistant()
      unsubscribePreferences()
      unsubscribeAgentProgress()
    }
  }, [])

  useEffect(() => {
    window.requestBrowser.app.getPreferences().then(setPreferences).catch(() => {})
  }, [])

  useEffect(() => {
    if (!preferences.showLlmChat) setAssistantOpen(false)
  }, [preferences.showLlmChat])

  useEffect(() => {
    const updateBounds = () => {
      const root = shellRef.current
      if (!root) return
      const rect = root.getBoundingClientRect()
      const appSurface = surface !== 'browser'
      const top = appSurface ? 56 : 96
      const left = appSurface ? 0 : sidebarCollapsed ? 56 : 256
      const right = assistantOpen ? 382 : 0
      window.requestBrowser.browser.setBounds({
        x: left,
        y: top,
        width: Math.max(360, rect.width - left - right),
        height: Math.max(240, rect.height - top - 28),
      })
    }
    updateBounds()
    void window.requestBrowser.app.setAssistantVisible(assistantOpen)
    const observer = new ResizeObserver(updateBounds)
    if (shellRef.current) observer.observe(shellRef.current)
    window.addEventListener('resize', updateBounds)
    return () => {
      observer.disconnect()
      window.removeEventListener('resize', updateBounds)
    }
  }, [assistantOpen, sidebarCollapsed, surface])

  const navigate = async () => {
    if (!address.trim()) return
    const next = await window.requestBrowser.browser.navigate(address.trim())
    setBrowser(next)
  }

  const openSurface = async (next: AppSurface) => {
    setSurface(next)
    await window.requestBrowser.app.openSurface(next)
  }

  const sendGoal = async (value = goal) => {
    const text = value.trim()
    if (!text || busy) return
    setBusy(true)
    setGoal('')
    setMessages((current) => [...current, { id: crypto.randomUUID(), role: 'user', text }])
    try {
      const result = await window.requestBrowser.agent.send(
        text,
        conversationIdRef.current,
      )
      setMessages((current) => [
        ...current,
        { id: crypto.randomUUID(), role: 'assistant', text: result.text },
      ])
      setSessions(await window.requestBrowser.workspace.listSessions())
      setDatabases(await window.requestBrowser.workspace.listDatabases())
    } catch (error) {
      setMessages((current) => [
        ...current,
        {
          id: crypto.randomUUID(),
          role: 'assistant',
          text: error instanceof Error ? error.message : 'The agent could not start.',
        },
      ])
    } finally {
      setBusy(false)
    }
  }

  const closeAssistant = () => {
    setAssistantOpen(false)
    const appSurface = surface !== 'browser'
    const top = appSurface ? 56 : 96
    const left = appSurface ? 0 : sidebarCollapsed ? 56 : 256
    setTimeout(() => window.requestBrowser.browser.setBounds({
      x: left,
      y: top,
      width: Math.max(360, window.innerWidth - left),
      height: Math.max(240, window.innerHeight - top - 28),
    }), 0)
  }

  return (
    <div ref={shellRef} className={`app-shell ${surface !== 'browser' ? 'app-surface' : ''}`}>
      <aside className={`rail ${sidebarCollapsed ? 'rail-collapsed' : ''}`}>
        <div className="rail-topline">
          <button className="icon-button" title="Toggle sidebar" onClick={() => setSidebarCollapsed((value) => !value)}>
            <Menu size={17} />
          </button>
          {!sidebarCollapsed && <span className="product-mark">Request Browser</span>}
        </div>
        <button className={`new-task ${sidebarCollapsed ? 'compact' : ''}`} onClick={() => void openSurface('home')} title="New research">
          <Plus size={16} />
          {!sidebarCollapsed && <span>New research</span>}
        </button>
        <nav className="rail-nav">
          <button className={`nav-item ${surface === 'browser' ? 'active' : ''}`} onClick={() => void openSurface('browser')}><Globe2 size={16} />{!sidebarCollapsed && <span>Browse</span>}</button>
          <button className={`nav-item ${surface === 'home' ? 'active' : ''}`} onClick={() => void openSurface('home')}><Sparkles size={16} />{!sidebarCollapsed && <span>Home</span>}</button>
          <button className={`nav-item ${surface === 'connect-apps' ? 'active' : ''}`} onClick={() => void openSurface('connect-apps')}><PlugZap size={16} />{!sidebarCollapsed && <span>Connect Apps</span>}</button>
          <button className={`nav-item ${surface === 'workspace' ? 'active' : ''}`} onClick={() => void openSurface('workspace')}><Database size={16} />{!sidebarCollapsed && <span>Workspace</span>}</button>
          <button className={`nav-item ${surface === 'bookmarks' ? 'active' : ''}`} onClick={() => void openSurface('bookmarks')}><BookOpen size={16} />{!sidebarCollapsed && <span>Bookmarks</span>}</button>
          <button className={`nav-item ${surface === 'history' ? 'active' : ''}`} onClick={() => void openSurface('history')}><Clock3 size={16} />{!sidebarCollapsed && <span>History</span>}</button>
          <button className={`nav-item ${surface === 'downloads' ? 'active' : ''}`} onClick={() => void openSurface('downloads')}><Download size={16} />{!sidebarCollapsed && <span>Downloads</span>}</button>
          <button className={`nav-item ${surface === 'scheduled' ? 'active' : ''}`} onClick={() => void openSurface('scheduled')}><CalendarClock size={16} />{!sidebarCollapsed && <span>Scheduled Tasks</span>}</button>
        </nav>
        {!sidebarCollapsed && (
          <div className="rail-section">
            <div className="rail-label">Recent goals</div>
            {sessions.slice(0, 4).map((session) => (
              <button key={session.id} className="recent-goal" onClick={() => void openSurface('workspace')}>
                <span className={`status-dot ${session.status}`} />
                <span>{session.goal}</span>
              </button>
            ))}
            {sessions.length === 0 && <div className="empty-rail">Your research threads appear here.</div>}
          </div>
        )}
        <div className="rail-bottom">
          <button className={`nav-item ${surface === 'settings' ? 'active' : ''}`} onClick={() => void openSurface('settings')}><Settings2 size={16} />{!sidebarCollapsed && <span>Settings</span>}</button>
          <button className="local-badge" title="Switch browser profile" onClick={() => void window.requestBrowser.app.showProfileMenu()}><span className="status-dot ready" />{!sidebarCollapsed && <span>{profile.name}</span>}<ChevronDown size={12} /></button>
        </div>
      </aside>

      <main className="main-stage">
        <header className="browser-toolbar">
          {surface !== 'browser' ? (
            <div className="app-surface-toolbar">
              <button className="back-to-browser" onClick={() => void openSurface('browser')}>
                <ChevronLeft size={16} />
                Back to browser
              </button>
              <span>{surface === 'connect-apps' ? 'Connect Apps' : surface === 'scheduled' ? 'Scheduled Tasks' : surface[0].toUpperCase() + surface.slice(1)}</span>
              {preferences.showLlmChat && <button className={`icon-button ${assistantOpen ? 'selected' : ''}`} title="Toggle assistant" onClick={() => setAssistantOpen((value) => !value)}><PanelRight size={16} /></button>}
            </div>
          ) : <>
          <div className="tab-strip" aria-label="Browser tabs">
            {tabs.map((tab) => (
              <button key={tab.id} className={`tab ${tab.active && surface === 'browser' ? 'active' : ''}`} onClick={() => { setSurface('browser'); void window.requestBrowser.browser.activateTab(tab.id) }} title={tab.title || tab.url}>
                {tab.faviconUrl ? <img className="tab-favicon" src={tab.faviconUrl} alt="" /> : <Globe2 size={13} />}<span>{tab.title || 'New tab'}</span>
                <X size={13} className="tab-close" onClick={(event) => { event.stopPropagation(); void window.requestBrowser.browser.closeTab(tab.id) }} />
              </button>
            ))}
            <button className="tab-add" title="New tab" onClick={() => { setSurface('browser'); void window.requestBrowser.browser.createTab() }}><Plus size={15} /></button>
          </div>
          <div className="toolbar-row">
            <div className="navigation-controls">
              <button className="icon-button" onClick={() => window.requestBrowser.browser.goBack()} disabled={!browser.canGoBack}><ArrowLeft size={16} /></button>
              <button className="icon-button" onClick={() => window.requestBrowser.browser.goForward()} disabled={!browser.canGoForward}><ArrowRight size={16} /></button>
              <button className="icon-button" onClick={() => window.requestBrowser.browser.reload()}><RefreshCw size={15} className={browser.loading ? 'spin' : ''} /></button>
            </div>
            <form className="address-form" onSubmit={(event) => { event.preventDefault(); void navigate() }}>
              {browser.url.startsWith('https://') ? <LockKeyhole size={14} /> : <Search size={15} />}
              <input ref={addressInputRef} value={address} onChange={(event) => setAddress(event.target.value)} aria-label="Address or search" />
              <span className="address-hint">Enter</span>
            </form>
            <div className="toolbar-actions">
              <button className="icon-button" title="Bookmark this page" onClick={() => void window.requestBrowser.browser.bookmarkCurrent()}><Star size={15} /></button>
              {preferences.showLlmChat && (
                <button className={`provider-chip ${preferences.showToolbarLabels ? '' : 'compact'}`} title="Choose AI provider" aria-label="Choose AI provider" onClick={() => { setActiveView('agent'); setAssistantOpen(true) }}><span className="provider-dot" />{preferences.showToolbarLabels && <><span>OpenCode</span><ChevronDown size={13} /></>}</button>
              )}
              {preferences.showLlmChat && <button className={`icon-button ${assistantOpen ? 'selected' : ''}`} title="Toggle assistant" onClick={() => setAssistantOpen((value) => !value)}><PanelRight size={16} /></button>}
              <button className="icon-button" title="More" onClick={() => void window.requestBrowser.app.showBrowserMenu()}><MoreHorizontal size={17} /></button>
            </div>
          </div>
          </>}
        </header>

        <div className="browser-stage">
          <div className="browser-status-pill"><span className={`status-dot ${browser.loading ? 'working' : 'ready'}`} />{browser.loading ? 'Loading page' : displayUrl.replace(/^https?:\/\//, '').slice(0, 80)}</div>
          {agentActivity && (
            <div className={`agent-live-status ${agentActivity.phase === 'error' ? 'error' : ''}`} role="status" aria-live="polite">
              <span className={`status-dot ${agentActivity.phase === 'completed' ? 'ready' : agentActivity.phase === 'error' ? 'error' : 'working'}`} />
              <span>{agentActivity.message}</span>
              {agentActivity.detail && <span className="agent-live-detail">{agentActivity.detail}</span>}
            </div>
          )}
        </div>
        <footer className="statusbar"><span>Request Browser</span><span className="status-separator" />{agentActivity?.message ?? (serverReady ? 'Agent server ready' : 'Starting local agent server…')}<span className="status-right">Session stays on this device</span></footer>
      </main>

      {false && assistantOpen && (
        <aside className="assistant-panel">
          <div className="assistant-header">
            <div><div className="eyebrow">Request Browser</div><h1>{activeView === 'agent' ? 'Research agent' : 'Workspace'}</h1></div>
            <button className="icon-button" title="Hide assistant" onClick={closeAssistant}><ChevronRight size={17} /></button>
          </div>
          <div className="assistant-tabs">
            <button className={activeView === 'agent' ? 'active' : ''} onClick={() => setActiveView('agent')}><Sparkles size={14} /> Agent</button>
            <button className={activeView === 'workspace' ? 'active' : ''} onClick={() => setActiveView('workspace')}><Database size={14} /> Workspace</button>
          </div>
          {activeView === 'agent' ? (
            <div className="agent-view">
              <div className="goal-card">
                <div className="goal-card-top"><span className="live-mark" /> LIVE GOAL <span className="goal-menu"><MoreHorizontal size={14} /></span></div>
                <div className="goal-title">Delegate a browser task to OpenCode</div>
                <div className="goal-subtitle">The agent can browse the current page, inspect sources, and save structured results to Workspace.</div>
                <div className="plan-row"><span className="plan-check done">✓</span><span>Connect to the local Request Browser agent</span></div>
                <div className="plan-row"><span className="plan-check" />Observe and navigate the current tab</div>
                <div className="plan-row"><span className="plan-check" />Extract and verify source-backed data</div>
              </div>
              <div className="message-list">
                {messages.length === 0 && <div className="agent-empty"><Sparkles size={19} /><p>Ask the agent to research what is open.</p><div className="suggestions"><button onClick={() => void sendGoal('Summarize the current page and save the source to my workspace.')}>Summarize this page</button><button onClick={() => void sendGoal('Extract the visible list into a workspace database.')}>Extract visible data</button></div></div>}
                {messages.map((message) => <div key={message.id} className={`message ${message.role}`}><div className="message-label">{message.role === 'user' ? 'You' : 'OpenCode'}</div><div>{message.text}</div></div>)}
                {busy && <div className="working-card"><LoaderCircle size={15} className="spin" /><span>OpenCode is working through the goal…</span></div>}
              </div>
              <form className="goal-composer" onSubmit={(event) => { event.preventDefault(); void sendGoal() }}>
                <textarea value={goal} onChange={(event) => setGoal(event.target.value)} placeholder="Tell the agent what to research…" rows={3} />
                <div className="composer-footer"><span><span className="provider-dot" /> OpenCode ACP</span><button type="submit" disabled={!goal.trim() || busy}>{busy ? <LoaderCircle size={15} className="spin" /> : <ArrowRight size={15} />} Run goal</button></div>
              </form>
            </div>
          ) : (
            <div className="workspace-view">
              <div className="workspace-intro"><Database size={18} /><div><strong>Your research workspace</strong><p>Saved records, sources, and assets from agent tasks.</p></div></div>
              <div className="workspace-label">Databases</div>
              {databases.map((database) => <button key={database.id} className="database-row"><Database size={15} /><span>{database.name}</span><span className="record-count">{database.recordCount}</span></button>)}
              {databases.length === 0 && <div className="empty-workspace">No databases yet. Run a goal and ask the agent to save extracted data.</div>}
              <button className="create-database"><Plus size={15} /> New database</button>
            </div>
          )}
        </aside>
      )}
    </div>
  )
}

createRoot(document.getElementById('root')!).render(<Shell />)
