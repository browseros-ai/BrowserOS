import { Activity, BookOpen } from 'lucide-react'
import type { ComponentType, SVGProps } from 'react'
import { useLocation, useNavigate } from 'react-router'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

export interface SidebarHelpProps {
  expanded?: boolean
}

interface HelpItem {
  name: string
  url: string
  internal?: boolean
  icon: ComponentType<SVGProps<SVGSVGElement>>
}

export const helpItems: HelpItem[] = [
  {
    name: 'Docs',
    url: 'https://docs.browseros.com/browserclaw',
    icon: BookOpen,
  },
  { name: 'Diagnostics', url: '/diagnostics', icon: Activity, internal: true },
]

/**
 * Opens an external help target in a new browser tab through the tabs API
 * (the extension holds the `tabs` permission), which also handles chrome://
 * targets that anchor navigations would block.
 */
export function openHelpTarget(url: string): void {
  chrome.tabs.create({ url })
}

export function SidebarHelp({ expanded = false }: SidebarHelpProps) {
  const navigate = useNavigate()
  const location = useLocation()
  return (
    <div className="overflow-hidden border-border border-t p-2">
      <div
        className={cn(
          'mb-1 truncate px-2.5 font-semibold text-[10px] text-muted-foreground uppercase tracking-[0.18em] transition-opacity duration-200',
          expanded ? 'opacity-100' : 'opacity-0',
        )}
      >
        Help
      </div>
      <div className="space-y-1">
        {helpItems.map((item) => {
          const Icon = item.icon

          const button = (
            <button
              type="button"
              onClick={() =>
                item.internal ? navigate(item.url) : openHelpTarget(item.url)
              }
              aria-current={
                item.internal && location.pathname === item.url
                  ? 'page'
                  : undefined
              }
              className={cn(
                item.internal &&
                  location.pathname === item.url &&
                  'bg-sidebar-accent text-sidebar-accent-foreground',
                'flex h-9 w-full items-center gap-3 overflow-hidden whitespace-nowrap rounded-md px-2.5 font-medium text-sm transition-colors hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
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
            </button>
          )

          if (!expanded) {
            return (
              <Tooltip key={item.url}>
                <TooltipTrigger render={button} />
                <TooltipContent side="right">{item.name}</TooltipContent>
              </Tooltip>
            )
          }

          return <div key={item.url}>{button}</div>
        })}
      </div>
    </div>
  )
}
