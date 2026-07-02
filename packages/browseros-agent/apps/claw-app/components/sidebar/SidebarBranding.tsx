import { cn } from '@/lib/utils'

export interface SidebarBrandingProps {
  expanded?: boolean
}

/**
 * Compact BrowserOS mark in the top of the sidebar. The deep-green square
 * with a "B" stays visible in the collapsed state; the full wordmark
 * appears as the sidebar expands. The wordmark fades rather than
 * sliding so the layout doesn't shift while the sidebar animates.
 */
export function SidebarBranding({ expanded = false }: SidebarBrandingProps) {
  return (
    <div className="flex h-14 shrink-0 items-center gap-3 px-3">
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-accent font-extrabold text-accent-foreground text-base shadow-card">
        B
      </span>
      <span
        className={cn(
          'truncate font-extrabold text-base tracking-tight transition-opacity duration-200',
          expanded ? 'opacity-100' : 'opacity-0',
        )}
      >
        BrowserOS
      </span>
    </div>
  )
}
