import { CalendarClock, Home, PlugZap, Settings } from 'lucide-react'
import type { FC } from 'react'
import { NavLink, useLocation } from 'react-router'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { Feature } from '@/lib/browseros/capabilities'
import { cn } from '@/lib/utils'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import { SidebarHistory } from './SidebarHistory'

export interface SidebarNavigationProps {
  expanded?: boolean
  onNavigate?: () => void
}

type NavItem = {
  name: string
  to: string
  icon: typeof Home
}

const primaryNavItems: NavItem[] = [
  { name: 'Home', to: '/home', icon: Home },
  {
    name: 'Connect Apps',
    to: '/connect-apps',
    icon: PlugZap,
  },
  { name: 'Scheduled Tasks', to: '/scheduled', icon: CalendarClock },
  {
    name: 'Settings',
    to: '/settings/ai',
    icon: Settings,
  },
]

function isNavItemActive(item: NavItem, pathname: string): boolean {
  if (item.to === '/settings/ai') {
    return pathname.startsWith('/settings')
  }

  return pathname === item.to
}

export const SidebarNavigation: FC<SidebarNavigationProps> = ({
  expanded = true,
  onNavigate,
}) => {
  const location = useLocation()
  const { supports } = useCapabilities()
  const showHistory = supports(Feature.NEWTAB_CHAT_HISTORY_SUPPORT)

  return (
    <TooltipProvider delayDuration={0}>
      <div className="flex-1 overflow-y-auto overflow-x-hidden p-2">
        <nav className="space-y-1">
          {primaryNavItems.map((item) => {
            const Icon = item.icon
            const isActive = isNavItemActive(item, location.pathname)

            const navItem = (
              <NavLink
                to={item.to}
                onClick={onNavigate}
                className={cn(
                  'flex h-9 items-center gap-2 overflow-hidden whitespace-nowrap rounded-md px-3 font-medium text-sm transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
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

            return (
              <div key={item.to}>
                {/* Expansion unmounts the content, so the trigger must own closing on pointer leave. */}
                <Tooltip disableHoverableContent>
                  <TooltipTrigger asChild>{navItem}</TooltipTrigger>
                  {!expanded && (
                    <TooltipContent side="right">{item.name}</TooltipContent>
                  )}
                </Tooltip>
                {/* Gate the mount so non-alpha navigation never starts history queries. */}
                {item.to === '/home' && showHistory && (
                  <SidebarHistory expanded={expanded} onNavigate={onNavigate} />
                )}
              </div>
            )
          })}
        </nav>
      </div>
    </TooltipProvider>
  )
}
