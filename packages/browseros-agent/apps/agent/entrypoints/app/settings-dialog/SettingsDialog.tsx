import {
  BookOpen,
  Bot,
  Compass,
  MessageSquare,
  Palette,
  RotateCcw,
  Search,
  Server,
  X,
} from 'lucide-react'
import type { FC } from 'react'
import { useEffect } from 'react'
import {
  type Location,
  useLocation,
  useNavigate,
  useParams,
} from 'react-router'
import { Dialog, DialogContent, DialogTitle } from '@/components/ui/dialog'
import { Feature } from '@/lib/browseros/capabilities'
import { useCapabilities } from '@/lib/browseros/useCapabilities'
import { SETTINGS_PAGE_VIEWED_EVENT } from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { cn } from '@/lib/utils'
import { AISettingsPage } from '../ai-settings/AISettingsPage'
import { CustomizationPage } from '../customization/CustomizationPage'
import { LlmHubPage } from '../llm-hub/LlmHubPage'
import { MCPSettingsPage } from '../mcp-settings/MCPSettingsPage'
import { SearchProviderPage } from '../search-provider/SearchProviderPage'

type SettingsTab = {
  id: string
  name: string
  icon: typeof Bot
  feature?: Feature
  component: FC
}

const settingsTabs: SettingsTab[] = [
  { id: 'ai', name: 'BrowserOS AI', icon: Bot, component: AISettingsPage },
  {
    id: 'chat',
    name: 'Chat & Council Provider',
    icon: MessageSquare,
    component: LlmHubPage,
  },
  {
    id: 'search',
    name: 'Search Provider',
    icon: Search,
    component: SearchProviderPage,
  },
  {
    id: 'customization',
    name: 'Customize BrowserOS',
    icon: Palette,
    feature: Feature.CUSTOMIZATION_SUPPORT,
    component: CustomizationPage,
  },
  {
    id: 'mcp',
    name: 'BrowserOS as MCP',
    icon: Server,
    component: MCPSettingsPage,
  },
]

type HelpItem = {
  name: string
  icon: typeof Bot
  href?: string
  to?: string
}

const helpItems: HelpItem[] = [
  { name: 'Docs', href: 'https://docs.browseros.com/', icon: BookOpen },
  { name: 'Features', to: '/onboarding/features', icon: Compass },
  { name: 'Revisit Onboarding', to: '/onboarding', icon: RotateCcw },
]

export const SettingsDialog: FC = () => {
  const { tab } = useParams<{ tab?: string }>()
  const location = useLocation()
  const navigate = useNavigate()
  const { supports } = useCapabilities()

  const backgroundLocation = (
    location.state as { backgroundLocation?: Location } | null
  )?.backgroundLocation

  const visibleTabs = settingsTabs.filter(
    (tabDef) => !tabDef.feature || supports(tabDef.feature),
  )

  const activeTab = visibleTabs.find((t) => t.id === tab) ? tab : 'ai'

  useEffect(() => {
    track(SETTINGS_PAGE_VIEWED_EVENT, { page: `settings/${activeTab}` })
  }, [activeTab])

  const handleClose = () => {
    if (backgroundLocation) {
      const target =
        backgroundLocation.pathname +
        (backgroundLocation.search || '') +
        (backgroundLocation.hash || '')
      navigate(target, { replace: true })
    } else {
      navigate('/home', { replace: true })
    }
  }

  const handleTabChange = (tabId: string) => {
    navigate(`/settings/${tabId}`, {
      state: { backgroundLocation },
      replace: true,
    })
  }

  const handleHelpNavigation = (to: string) => {
    navigate(to, { replace: true })
  }

  const activeTabConfig = visibleTabs.find((t) => t.id === activeTab)
  const ActiveComponent = activeTabConfig?.component ?? AISettingsPage

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) handleClose()
      }}
    >
      <DialogContent
        className="flex h-[85vh] max-h-[85vh] w-full flex-col gap-0 overflow-hidden p-0 sm:max-w-4xl"
        showCloseButton={false}
      >
        <DialogTitle className="sr-only">Settings</DialogTitle>
        <div className="flex h-full min-h-0">
          {/* Left panel - tab navigation */}
          <div className="flex w-52 shrink-0 flex-col border-r bg-muted/30">
            <div className="px-4 pt-5 pb-3">
              <span className="font-semibold text-muted-foreground text-xs uppercase tracking-wider">
                Settings
              </span>
            </div>
            <nav className="flex-1 space-y-0.5 overflow-y-auto px-2">
              {visibleTabs.map((tabDef) => {
                const Icon = tabDef.icon
                return (
                  <button
                    key={tabDef.id}
                    type="button"
                    onClick={() => handleTabChange(tabDef.id)}
                    className={cn(
                      'flex w-full items-center gap-2 rounded-md px-3 py-2 font-medium text-sm transition-colors hover:bg-accent hover:text-accent-foreground',
                      activeTab === tabDef.id &&
                        'bg-accent text-accent-foreground',
                    )}
                  >
                    <Icon className="size-4 shrink-0" />
                    <span className="truncate">{tabDef.name}</span>
                  </button>
                )
              })}
            </nav>

            {/* Help section */}
            <div className="border-t px-2 py-2">
              <div className="mb-1 px-3 font-semibold text-[10px] text-muted-foreground uppercase tracking-[0.18em]">
                Help
              </div>
              {helpItems.map((item) => {
                const Icon = item.icon
                if (item.href) {
                  return (
                    <a
                      key={item.name}
                      href={item.href}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="flex w-full items-center gap-2 rounded-md px-3 py-2 font-medium text-sm transition-colors hover:bg-accent hover:text-accent-foreground"
                    >
                      <Icon className="size-4 shrink-0" />
                      <span className="truncate">{item.name}</span>
                    </a>
                  )
                }
                return (
                  <button
                    key={item.name}
                    type="button"
                    onClick={() => handleHelpNavigation(item.to ?? '/home')}
                    className="flex w-full items-center gap-2 rounded-md px-3 py-2 font-medium text-sm transition-colors hover:bg-accent hover:text-accent-foreground"
                  >
                    <Icon className="size-4 shrink-0" />
                    <span className="truncate">{item.name}</span>
                  </button>
                )
              })}
            </div>
          </div>

          {/* Right panel - settings content */}
          <div className="flex flex-1 flex-col overflow-hidden">
            <div className="flex justify-end px-4 pt-3">
              <button
                type="button"
                onClick={handleClose}
                className="rounded-sm opacity-70 ring-offset-background transition-opacity hover:opacity-100"
              >
                <X className="size-4" />
                <span className="sr-only">Close</span>
              </button>
            </div>
            <div className="styled-scrollbar flex-1 overflow-y-auto px-6 pb-6">
              <ActiveComponent />
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
