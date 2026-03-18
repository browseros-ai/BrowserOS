import {
  Brain,
  CalendarClock,
  GitBranch,
  Home,
  PlugZap,
  Settings,
  Sparkles,
  Wand2,
} from 'lucide-react'
import type { FC } from 'react'
import { NavLink, useLocation } from 'react-router'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { Feature } from '@/lib/browseros/capabilities'
import { useCapabilities } from '@/lib/browseros/useCapabilities'
import { useOpenSettings } from '@/lib/settings/useOpenSettings'
import { cn } from '@/lib/utils'

interface SidebarNavigationProps {
  expanded?: boolean
}

type NavItem = {
  name: string
  to?: string
  icon: typeof Home
  feature?: Feature
  action?: 'settings'
}

const primaryNavItems: NavItem[] = [
  { name: 'Home', to: '/home', icon: Home },
  {
    name: 'Connect Apps',
    to: '/connect-apps',
    icon: PlugZap,
    feature: Feature.MANAGED_MCP_SUPPORT,
  },
  { name: 'Scheduled Tasks', to: '/scheduled', icon: CalendarClock },
  {
    name: 'Workflows',
    to: '/workflows',
    icon: GitBranch,
    feature: Feature.WORKFLOW_SUPPORT,
  },
  {
    name: 'Skills',
    to: '/home/skills',
    icon: Wand2,
    feature: Feature.SKILLS_SUPPORT,
  },
  {
    name: 'Memory',
    to: '/home/memory',
    icon: Brain,
    feature: Feature.MEMORY_SUPPORT,
  },
  {
    name: 'Soul',
    to: '/home/soul',
    icon: Sparkles,
    feature: Feature.SOUL_SUPPORT,
  },
  { name: 'Settings', icon: Settings, action: 'settings' },
]

const navItemClassName =
  'flex h-9 items-center gap-2 overflow-hidden whitespace-nowrap rounded-md px-3 font-medium text-sm transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground'

export const SidebarNavigation: FC<SidebarNavigationProps> = ({
  expanded = true,
}) => {
  const location = useLocation()
  const openSettings = useOpenSettings()
  const { supports } = useCapabilities()
  const isSettingsActive = location.pathname.startsWith('/settings')

  const filteredItems = primaryNavItems.filter(
    (item) => !item.feature || supports(item.feature),
  )

  return (
    <TooltipProvider delayDuration={0}>
      <div className="flex-1 overflow-y-auto overflow-x-hidden p-2">
        <nav className="space-y-1">
          {filteredItems.map((item) => {
            const Icon = item.icon

            // Settings is a button that opens the dialog
            if (item.action === 'settings') {
              const settingsButton = (
                <button
                  type="button"
                  onClick={() => openSettings()}
                  className={cn(
                    navItemClassName,
                    'w-full',
                    isSettingsActive &&
                      'bg-sidebar-accent text-sidebar-accent-foreground',
                  )}
                >
                  <Icon className="size-4 shrink-0" />
                  <span
                    className={cn(
                      'truncate transition-opacity duration-200',
                      expanded ? 'opacity-100' : 'opacity-0',
                    )}
                  >
                    {item.name}
                  </span>
                </button>
              )

              if (!expanded) {
                return (
                  <Tooltip key="settings">
                    <TooltipTrigger asChild>{settingsButton}</TooltipTrigger>
                    <TooltipContent side="right">{item.name}</TooltipContent>
                  </Tooltip>
                )
              }

              return <div key="settings">{settingsButton}</div>
            }

            // Regular nav items use NavLink
            const itemPath = item.to ?? '/home'
            const isActive = location.pathname === itemPath
            const navItem = (
              <NavLink
                to={itemPath}
                className={cn(
                  navItemClassName,
                  isActive &&
                    'bg-sidebar-accent text-sidebar-accent-foreground',
                )}
              >
                <Icon className="size-4 shrink-0" />
                <span
                  className={cn(
                    'truncate transition-opacity duration-200',
                    expanded ? 'opacity-100' : 'opacity-0',
                  )}
                >
                  {item.name}
                </span>
              </NavLink>
            )

            if (!expanded) {
              return (
                <Tooltip key={item.to}>
                  <TooltipTrigger asChild>{navItem}</TooltipTrigger>
                  <TooltipContent side="right">{item.name}</TooltipContent>
                </Tooltip>
              )
            }

            return <div key={item.to}>{navItem}</div>
          })}
        </nav>
      </div>
    </TooltipProvider>
  )
}
