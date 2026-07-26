import type { ComponentType, ReactNode, SVGProps } from 'react'

interface PlaceholderScreenProps {
  icon: ComponentType<SVGProps<SVGSVGElement>>
  title: string
  description: ReactNode
}

/**
 * Holding pattern for routes that exist for navigation hygiene but
 * have not been built yet. Renders the route name, a one-liner so
 * the user understands the scope, and a small icon so the visual
 * weight matches the cockpit page.
 */
export function PlaceholderScreen({
  icon: Icon,
  title,
  description,
}: PlaceholderScreenProps) {
  return (
    <div className="mx-auto flex max-w-5xl flex-col items-start gap-3 px-8 pt-24">
      <span className="flex h-10 w-10 items-center justify-center rounded-lg bg-accent-tint text-accent-ink">
        <Icon className="size-5" />
      </span>
      <h1 className="font-extrabold text-3xl tracking-tight">{title}</h1>
      <p className="max-w-xl text-ink-3 text-sm leading-relaxed">
        {description}
      </p>
      <span className="mt-4 inline-flex items-center gap-2 rounded-full bg-bg-sunken px-3 py-1 text-ink-3 text-xs">
        Coming soon
      </span>
    </div>
  )
}
