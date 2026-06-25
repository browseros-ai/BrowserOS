import { User } from 'lucide-react'
import { Checkbox } from '@/components/ui/checkbox'
import { cn } from '@/lib/utils'
import type { ChromeProfile } from '../onboarding-v2.helpers'

interface ChromeProfileTileProps {
  profile: ChromeProfile
  checked: boolean
  onCheckedChange: (next: boolean) => void
}

/**
 * One Chrome profile row in the picker. Click-to-toggle on the whole
 * tile per the project's row-toggle convention. Checked tiles border
 * in accent orange with an accent-tint background; unchecked tiles
 * stay neutral. The shadcn `Checkbox` primitive is the visible
 * checkmark; the row is layout around it.
 */
export function ChromeProfileTile({
  profile,
  checked,
  onCheckedChange,
}: ChromeProfileTileProps) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: a button with role=checkbox is the row-toggle pattern; the design demands the entire tile is the click target, not just a checkbox primitive
    <button
      type="button"
      role="checkbox"
      aria-checked={checked}
      onClick={() => onCheckedChange(!checked)}
      className={cn(
        'flex w-full items-center gap-3 rounded-xl border px-4 py-3 text-left transition-colors',
        checked
          ? 'border-accent bg-accent-tint'
          : 'border-border-2 bg-card hover:border-border-strong',
      )}
    >
      <Checkbox
        checked={checked}
        // Click on the underlying primitive is swallowed by the outer
        // button's onClick; this still keeps the visual primitive in
        // sync via the `checked` prop.
        tabIndex={-1}
        aria-hidden
      />
      <span className="flex size-[30px] shrink-0 items-center justify-center rounded-lg border border-border-2 bg-card text-ink-2">
        <User className="size-[15px]" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="font-bold text-[13.5px] text-ink">{profile.name}</div>
        <div className="truncate text-[11.5px] text-ink-3">{profile.email}</div>
      </div>
      <div className="shrink-0 text-right font-mono text-[11.5px] text-ink-2">
        {profile.sites} sites . {profile.logins} logins
      </div>
    </button>
  )
}
