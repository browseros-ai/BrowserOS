import { CheckCircle2, Circle, User } from 'lucide-react'
import { cn } from '@/lib/utils'
import type { BrowserOSImportSource } from '../browseros-onboarding-api'

interface ImportSourceTileProps {
  source: BrowserOSImportSource
  selected: boolean
  onSelect: () => void
}

/** Renders one Chromium import source option in the picker. */
export function ImportSourceTile({
  source,
  selected,
  onSelect,
}: ImportSourceTileProps) {
  return (
    <label
      className={cn(
        'flex w-full cursor-pointer items-center gap-3 rounded-xl border px-4 py-2.5 text-left transition-colors has-[:focus-visible]:ring-2 has-[:focus-visible]:ring-accent has-[:focus-visible]:ring-offset-2',
        selected
          ? 'border-accent bg-accent-tint'
          : 'border-border-2 bg-card hover:border-border-strong',
      )}
    >
      <input
        type="radio"
        name="browseros-import-source"
        checked={selected}
        onChange={onSelect}
        className="sr-only"
      />
      {selected ? (
        <CheckCircle2 className="size-4 shrink-0 text-accent" />
      ) : (
        <Circle className="size-4 shrink-0 text-ink-4" />
      )}
      <span className="flex size-[30px] shrink-0 items-center justify-center rounded-lg border border-border-2 bg-card text-ink-2">
        <User className="size-[15px]" />
      </span>
      <div className="min-w-0 flex-1">
        <div className="font-bold text-[13.5px] text-ink">
          {source.profileName || source.displayName}
        </div>
        <div className="truncate text-[11.5px] text-ink-3">
          {source.browserName}
        </div>
      </div>
    </label>
  )
}
