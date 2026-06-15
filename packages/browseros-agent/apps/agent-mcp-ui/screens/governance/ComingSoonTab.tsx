import type { ComponentType, SVGProps } from 'react'

interface ComingSoonTabProps {
  icon: ComponentType<SVGProps<SVGSVGElement>>
  title: string
  description: string
}

/**
 * Lightweight stub used by Governance tabs that haven't shipped yet.
 * Smaller than PlaceholderScreen since the tab header sits above it
 * and we don't want to double-stack the visual hierarchy.
 */
export function ComingSoonTab({
  icon: Icon,
  title,
  description,
}: ComingSoonTabProps) {
  return (
    <div className="flex flex-col items-start gap-3 rounded-2xl border border-border border-dashed bg-card px-6 py-10">
      <span className="flex size-9 items-center justify-center rounded-lg bg-accent-tint text-accent-ink">
        <Icon className="size-4" />
      </span>
      <h2 className="font-bold text-ink text-lg tracking-tight">{title}</h2>
      <p className="max-w-md text-ink-3 text-sm leading-snug">{description}</p>
      <span className="mt-1 inline-flex items-center gap-1.5 rounded-full bg-bg-sunken px-2.5 py-0.5 text-ink-3 text-xs">
        Coming soon
      </span>
    </div>
  )
}
