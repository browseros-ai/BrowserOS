import { Ban, Check, type LucideIcon, ShieldQuestion } from 'lucide-react'
import { cn } from '@/lib/utils'
import {
  APPROVAL_CATEGORIES,
  type ApprovalCategory,
  type ApprovalVerdict,
} from '@/screens/new-agent/new-agent.schemas'

/**
 * Read-only catalog: every action an agent can take, grouped by the
 * default verdict it inherits. The new-agent wizard overrides per
 * agent; this is the system-wide default each agent starts from.
 */
export function PermissionsTab() {
  const buckets = bucketise(APPROVAL_CATEGORIES)
  return (
    <section className="flex flex-col gap-4">
      <p className="text-ink-2 text-sm leading-snug">
        The default action catalog every new agent inherits. The wizard
        overrides per agent in the Tool approvals section; the buckets here set
        the floor.
      </p>
      <div className="grid grid-cols-1 gap-3 lg:grid-cols-3">
        {VERDICT_ORDER.map((verdict) => (
          <BucketCard
            key={verdict}
            verdict={verdict}
            categories={buckets[verdict]}
          />
        ))}
      </div>
    </section>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components and helpers, private to this tab.
 * -------------------------------------------------------------------------*/

const VERDICT_ORDER: readonly ApprovalVerdict[] = ['Auto', 'Ask', 'Block']

interface VerdictStyle {
  label: string
  caption: string
  Icon: LucideIcon
  cardClass: string
  badgeClass: string
  dotClass: string
}

const VERDICT_STYLE: Record<ApprovalVerdict, VerdictStyle> = {
  Auto: {
    label: 'Auto',
    caption: 'Safe by default. Agents run these without asking.',
    Icon: Check,
    cardClass: 'border-green-tint',
    badgeClass: 'bg-green-tint text-green',
    dotClass: 'bg-green',
  },
  Ask: {
    label: 'Ask',
    caption: 'Approval card pops up in the activity panel.',
    Icon: ShieldQuestion,
    cardClass: 'border-amber-tint',
    badgeClass: 'bg-amber-tint text-amber',
    dotClass: 'bg-amber',
  },
  Block: {
    label: 'Block',
    caption: 'Refused outright at the browser; never asks.',
    Icon: Ban,
    cardClass: 'border-red-tint',
    badgeClass: 'bg-red-tint text-red',
    dotClass: 'bg-red',
  },
}

function bucketise(
  categories: readonly ApprovalCategory[],
): Record<ApprovalVerdict, ApprovalCategory[]> {
  const acc: Record<ApprovalVerdict, ApprovalCategory[]> = {
    Auto: [],
    Ask: [],
    Block: [],
  }
  for (const category of categories) {
    acc[category.defaultVerdict].push(category)
  }
  return acc
}

interface BucketCardProps {
  verdict: ApprovalVerdict
  categories: readonly ApprovalCategory[]
}

function BucketCard({ verdict, categories }: BucketCardProps) {
  const style = VERDICT_STYLE[verdict]
  return (
    <article
      className={cn(
        'flex flex-col gap-2.5 rounded-2xl border bg-card p-4',
        style.cardClass,
      )}
    >
      <header className="flex items-center gap-2">
        <span
          className={cn(
            'flex size-6 items-center justify-center rounded-md',
            style.badgeClass,
          )}
        >
          <style.Icon className="size-3.5" />
        </span>
        <span className="font-bold text-ink text-sm">{style.label}</span>
        <span className="ml-auto font-mono text-ink-3 text-xs">
          {categories.length}
        </span>
      </header>
      <p className="text-ink-3 text-xs leading-snug">{style.caption}</p>
      <ul className="flex flex-col gap-1.5">
        {categories.map((category) => (
          <li
            key={category.id}
            className="flex items-center gap-2 rounded-lg bg-bg-sunken px-2.5 py-1.5 text-ink text-xs"
          >
            <span className={cn('size-1.5 rounded-full', style.dotClass)} />
            {category.name}
          </li>
        ))}
        {categories.length === 0 && (
          <li className="rounded-lg border border-border-2 border-dashed px-2.5 py-2 text-center text-ink-4 text-xs">
            None in this bucket
          </li>
        )}
      </ul>
    </article>
  )
}
