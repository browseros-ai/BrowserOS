import { CalendarClock, Home, PlugZap, Settings } from 'lucide-react'
import type { FC } from 'react'
import { NavLink, useLocation } from 'react-router'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

export interface SidebarNavigationProps {
  expanded?: boolean
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
}) => {
  const location = useLocation()

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
