import { cn } from '@/lib/utils'

export interface SidebarBrandingProps {
  expanded?: boolean
}

/**
 * Compact BrowserOS neo mark in the top of the sidebar. The icon (a
 * rounded-square with the blue claw glyph) stays visible in the
 * collapsed state; the full wordmark appears as the sidebar expands.
 * The wordmark fades rather than sliding so the layout does not shift
 * while the sidebar animates.
 *
 * Two assets, not one: the mark ships as an <img>, so no token, no
 * dark: utility and no currentColor reaches inside it, and the light
 * asset's outer plate is a hard fill="#fff" that would sit as a white
 * tile on the gray rail. Same swap idiom as components/harness/
 * harness-marks.tsx; only one of the pair is ever displayed, so both
 * carry the same alt text.
 */
export function SidebarBranding({ expanded = false }: SidebarBrandingProps) {
  return (
    <div className="flex h-14 shrink-0 items-center gap-3 px-3">
      <img
        src="/icons/browserclaw.svg"
        alt="BrowserOS neo"
        className="size-8 shrink-0 rounded-md shadow-card dark:hidden"
      />
      <img
        src="/icons/browserclaw-dark.svg"
        alt="BrowserOS neo"
        className="hidden size-8 shrink-0 rounded-md shadow-card dark:block"
      />
      <span
        className={cn(
          'truncate font-extrabold text-base tracking-tight transition-opacity duration-200',
          expanded ? 'opacity-100' : 'opacity-0',
        )}
      >
        BrowserOS neo
      </span>
    </div>
  )
}
