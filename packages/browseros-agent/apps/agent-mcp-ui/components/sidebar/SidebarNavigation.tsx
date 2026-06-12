import { Bot, LayoutDashboard, PlugZap, ShieldCheck } from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
import { NavLink, useLocation } from 'react-router'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

export interface SidebarNavigationProps {
  expanded?: boolean
}

interface NavItem {
  name: string
  to: string
  icon: ComponentType<SVGProps<SVGSVGElement>>
}

const navItems: NavItem[] = [
  { name: 'Cockpit', to: '/', icon: LayoutDashboard },
  { name: 'Agents', to: '/agents', icon: Bot },
  { name: 'Governance', to: '/governance', icon: ShieldCheck },
  { name: 'MCP', to: '/mcp', icon: PlugZap },
]

/**
 * `/agents/new` lives under `/agents` so the Agents item should stay
 * highlighted when the new-agent wizard is open. `/` is otherwise an
 * exact match. The rest use simple `startsWith` so future nested
 * routes (e.g. `/governance/audit`) still highlight correctly.
 */
function isNavItemActive(item: NavItem, pathname: string): boolean {
  if (item.to === '/') return pathname === '/'
  return pathname.startsWith(item.to)
}

export function SidebarNavigation({ expanded = true }: SidebarNavigationProps) {
  const location = useLocation()

  return (
    <nav className="flex-1 overflow-y-auto overflow-x-hidden p-2">
      <div className="space-y-1">
        {navItems.map((item) => {
          const Icon = item.icon
          const isActive = isNavItemActive(item, location.pathname)

          const navLink = (
            <NavLink
              to={item.to}
              className={cn(
                'flex h-9 items-center gap-3 overflow-hidden whitespace-nowrap rounded-md px-2.5 font-medium text-sm transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                isActive && 'bg-sidebar-accent text-sidebar-accent-foreground',
              )}
            >
              <Icon className="size-5 shrink-0" />
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
                <TooltipTrigger render={navLink} />
                <TooltipContent side="right">{item.name}</TooltipContent>
              </Tooltip>
            )
          }

          return <div key={item.to}>{navLink}</div>
        })}
      </div>
    </nav>
  )
}
