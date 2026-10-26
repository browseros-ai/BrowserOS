import { History, Search, TriangleAlert } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button, ButtonLink } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import { cn } from '@/lib/utils'

type AuditEmptyProps =
  | { variant: 'zero-tasks' }
  | { variant: 'error' }
  | { variant: 'search-miss'; search: string; onShowAll: () => void }

/** What History says when there is nothing to list, and the one way forward. */
export function AuditEmpty(props: AuditEmptyProps) {
  if (props.variant === 'zero-tasks') {
    return (
      <EmptyCard
        icon={<History className="size-5" />}
        title="No tasks yet"
        hint="When your AI agents use neo, each task is saved here so you can see what it did."
        action={
          <ButtonLink href="#/mcp" variant="secondary" size="sm">
            Connect an agent
          </ButtonLink>
        }
      />
    )
  }
  if (props.variant === 'error') {
    return (
      <EmptyCard
        tone="error"
        icon={<TriangleAlert className="size-5" />}
        title="Couldn’t load your history"
        hint="Check that BrowserOS is running, then open this page again."
      />
    )
  }
  return (
    <EmptyCard
      icon={<Search className="size-5" />}
      title={
        props.search
          ? `No tasks match “${props.search}”`
          : 'No tasks match these filters'
      }
      hint="Try a shorter word, or show all tasks."
      action={
        <Button variant="secondary" size="sm" onClick={props.onShowAll}>
          Show all tasks
        </Button>
      }
    />
  )
}

// ---------------------------------------------------------------------------

function EmptyCard({
  icon,
  title,
  hint,
  action,
  tone = 'neutral',
}: {
  icon: ReactNode
  title: string
  hint: string
  action?: ReactNode
  tone?: 'neutral' | 'error'
}) {
  return (
    <Card
      size="compact"
      className="flex-row flex-wrap items-center gap-4 border-border-2 bg-card px-[22px] py-5 shadow-card"
    >
      <span
        aria-hidden
        className={cn(
          'grid size-11 shrink-0 place-items-center rounded-full',
          tone === 'error'
            ? 'bg-red-tint text-red-ink'
            : 'bg-accent-tint text-primary',
        )}
      >
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="font-bold text-[16px] text-cyanotype-ink">
          {title}
        </span>
        <span className="text-[14.5px] text-ink-2">{hint}</span>
      </span>
      {action}
    </Card>
  )
}
