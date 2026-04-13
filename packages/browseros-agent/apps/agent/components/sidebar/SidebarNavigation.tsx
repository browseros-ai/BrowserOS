import {
  Brain,
  CalendarClock,
  Cpu,
  Home,
  PlugZap,
  Settings,
  Shield,
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
import { cn } from '@/lib/utils'

interface SidebarNavigationProps {
  expanded?: boolean
}

type NavItem = {
  name: string
  to: string
  icon: typeof Home
  feature?: Feature
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
  { name: 'Agents', to: '/agents', icon: Cpu },
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
  { name: 'Governance', to: '/admin', icon: Shield },
  { name: 'Settings', to: '/settings/ai', icon: Settings },
]

export const SidebarNavigation: FC<SidebarNavigationProps> = ({
  expanded = true,
}) => {
  const location = useLocation()
  const { supports } = useCapabilities()

  const filteredItems = primaryNavItems.filter(
    (item) => !item.feature || supports(item.feature),
  )

  return (
    <TooltipProvider delayDuration={0}>
      <div className="flex-1 overflow-y-auto overflow-x-hidden p-2">
        <nav className="space-y-1">
          {filteredItems.map((item) => {
            const Icon = item.icon
            const isActive =
              item.to === '/settings/ai'
                ? location.pathname.startsWith('/settings')
                : location.pathname === item.to

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
