import { cn } from '@/lib/utils'

export interface SidebarBrandingProps {
  expanded?: boolean
}

/**
 * Compact BrowserClaw mark in the top of the sidebar. The icon (a
 * rounded-square with the blue claw glyph on white) stays visible in
 * the collapsed state; the full wordmark appears as the sidebar
 * expands. The wordmark fades rather than sliding so the layout does
 * not shift while the sidebar animates.
 */
export function SidebarBranding({ expanded = false }: SidebarBrandingProps) {
  return (
    <div className="flex h-14 shrink-0 items-center gap-3 px-3">
      <img
        src="/icons/browserclaw.svg"
        alt="BrowserClaw"
        className="size-8 shrink-0 rounded-md shadow-card"
      />
      <span
        className={cn(
          'truncate font-extrabold text-base tracking-tight transition-opacity duration-200',
          expanded ? 'opacity-100' : 'opacity-0',
        )}
      >
        BrowserClaw
      </span>
    </div>
  )
}
