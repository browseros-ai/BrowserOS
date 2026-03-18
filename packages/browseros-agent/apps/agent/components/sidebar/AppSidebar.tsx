import type { FC } from 'react'
import { cn } from '@/lib/utils'
import { SidebarBranding } from './SidebarBranding'
import { SidebarNavigation } from './SidebarNavigation'
import { SidebarUserFooter } from './SidebarUserFooter'

interface AppSidebarProps {
  expanded?: boolean
  onOpenShortcuts?: () => void
}

export const AppSidebar: FC<AppSidebarProps> = ({
  expanded = false,
  onOpenShortcuts,
}) => {
  return (
    <div
      className={cn(
        'flex h-full flex-col border-r bg-sidebar text-sidebar-foreground transition-all duration-200 ease-in-out',
        expanded ? 'w-64' : 'w-14',
      )}
    >
      <SidebarBranding expanded={expanded} />
      <SidebarNavigation expanded={expanded} />
      <SidebarUserFooter
        expanded={expanded}
        onOpenShortcuts={onOpenShortcuts}
      />
    </div>
  )
}
