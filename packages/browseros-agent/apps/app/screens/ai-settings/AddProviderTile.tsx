import type { FC, ReactNode } from 'react'

export interface AddProviderTileProps {
  label: string
  icon: ReactNode
  onAdd: () => void
}

/**
 * One entry in the add-a-provider grid. Every entry uses this, so the action
 * reads the same whether it opens a key form, an OAuth flow or an agent
 * dialog. The whole tile is the button; the visible "Add" is an affordance
 * rather than a separate target, which keeps one tab stop per provider.
 */
export const AddProviderTile: FC<AddProviderTileProps> = ({
  label,
  icon,
  onAdd,
}) => (
  <button
    type="button"
    onClick={onAdd}
    aria-label={`Add ${label}`}
    className="group flex w-full items-center gap-3 rounded-lg border border-border bg-card p-3 text-left transition-all hover:-translate-y-px hover:border-[var(--accent-orange)]"
  >
    <span className="flex size-7 shrink-0 items-center justify-center text-accent-orange/70 transition-colors group-hover:text-accent-orange">
      {icon}
    </span>
    <span className="min-w-0 flex-1 truncate font-medium text-foreground text-sm">
      {label}
    </span>
    <span
      aria-hidden
      className="shrink-0 rounded-md border border-border bg-secondary px-2.5 py-1 font-semibold text-foreground text-xs transition-colors group-hover:border-[var(--accent-orange)] group-hover:bg-[var(--accent-orange)] group-hover:text-white"
    >
      Add
    </span>
  </button>
)
