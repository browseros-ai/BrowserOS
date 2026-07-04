import { cn } from '@/lib/utils'
import { SidebarBranding } from './SidebarBranding'
import { SidebarNavigation } from './SidebarNavigation'

export interface AppSidebarProps {
  expanded?: boolean
}

/**
 * Wraps the branding + navigation. Room for a footer strip lands in a
 * follow-up when there is a real setting to surface.
 */
export function AppSidebar({ expanded = false }: AppSidebarProps) {
  return (
    <div
      className={cn(
        'flex h-full flex-col border-border border-r bg-sidebar text-sidebar-foreground transition-all duration-200 ease-in-out',
        expanded ? 'w-64' : 'w-14',
      )}
    >
      <SidebarBranding expanded={expanded} />
      <SidebarNavigation expanded={expanded} />
    </div>
  )
}
