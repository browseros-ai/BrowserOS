import { cn } from '@/lib/utils'
import { SidebarBranding } from './SidebarBranding'
import { SidebarHelp } from './SidebarHelp'
import { SidebarNavigation } from './SidebarNavigation'
import { SidebarPrivacy } from './SidebarPrivacy'

export interface AppSidebarProps {
  expanded?: boolean
}

/** Sidebar shell: branding, primary navigation, and a bottom help footer. */
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
      <div className="px-2 pb-1">
        <SidebarPrivacy expanded={expanded} />
      </div>
      <SidebarHelp expanded={expanded} />
    </div>
  )
}
