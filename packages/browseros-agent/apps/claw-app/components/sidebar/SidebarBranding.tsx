import { cn } from '@/lib/utils'

export interface SidebarBrandingProps {
  expanded?: boolean
}

/**
 * Compact BrowserOS neo mark in the top of the sidebar. The icon (a blue
 * rounded-square tile with the white claw glyph) stays visible in the
 * collapsed state; the full wordmark appears as the sidebar expands.
 * The wordmark fades rather than sliding so the layout does not shift
 * while the sidebar animates.
 *
 * ONE ASSET, BOTH THEMES, and that is deliberate. The mark is a brand
 * chip, not a themed surface: Chrome, Slack and VS Code do not re-tint
 * their own marks in dark mode, and a bright chip on the dark rail is
 * the intended reading. Same call the palette already makes for
 * --mcp-endpoint, which holds #0454ec in both themes.
 *
 * #2406 briefly shipped a browserclaw-dark.svg behind a dark:hidden /
 * dark:block pair. Re-tinting inverted the mark: the white fill in this
 * asset is the DOG, showing through a hole in the blue tile path, not a
 * backing plate. Darkening it to #303333 and lifting the tile to #4d8dff
 * produced a dark dog on a pale tile — 3.99:1 glyph-on-tile, 1.42:1
 * glyph-against the #141616 rail — which is the murky square that got
 * reported. The single light asset scores 6.49:1 and 18.16:1 there.
 *
 * components/harness/harness-marks.tsx keeps the dark:hidden idiom and
 * should: those are third-party logos (Claude, Codex, Cursor) that
 * genuinely ship per-theme variants. This rule is about our own mark.
 */
export function SidebarBranding({ expanded = false }: SidebarBrandingProps) {
  return (
    <div className="flex h-14 shrink-0 items-center gap-3 px-3">
      <img
        src="/icons/browserclaw.svg"
        alt="BrowserOS neo"
        className="size-8 shrink-0 rounded-md shadow-card"
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
