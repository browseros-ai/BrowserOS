import { Trash2 } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { cn } from '@/lib/utils'
import type {
  AddSiteRuleVariables,
  SiteRule,
} from '@/modules/api/site-rules.hooks'
import { AddSiteRuleForm } from './AddSiteRuleForm'
import { useSiteRulesData } from './site-rules.data'
import {
  siteRuleActionBadgeClass,
  siteRuleActionLabel,
} from './site-rules.helpers'

export function SiteRulesTab() {
  const { rules, isLoading, addRule, deleteRule } = useSiteRulesData()

  const onAdd = (
    values: AddSiteRuleVariables,
    options?: { onSuccess?: () => void },
  ) => addRule.mutate(values, options)
  const onDelete = (rule: SiteRule) => deleteRule.mutate({ id: rule.id })

  return (
    <section className="flex flex-col gap-3">
      <header className="flex items-start justify-between gap-3">
        <p className="max-w-[60ch] text-ink-2 text-sm leading-snug">
          Per-domain blocks the browser enforces directly, even under prompt
          injection. Each rule clamps one action category on one domain pattern
          for every agent.
        </p>
        <span className="shrink-0 rounded-full bg-bg-sunken px-2.5 py-1 font-mono text-ink-3 text-xs">
          {rules.length} rule{rules.length === 1 ? '' : 's'}
        </span>
      </header>

      {isLoading ? (
        <div className="flex justify-center py-12 text-ink-3">
          <Spinner />
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          {rules.map((rule) => (
            <SiteRuleRow
              key={rule.id}
              rule={rule}
              isDeleting={
                deleteRule.isPending && deleteRule.variables?.id === rule.id
              }
              onDelete={onDelete}
            />
          ))}
          <AddSiteRuleForm isSubmitting={addRule.isPending} onSubmit={onAdd} />
        </div>
      )}
    </section>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, kept private to the tab.
 * -------------------------------------------------------------------------*/

interface SiteRuleRowProps {
  rule: SiteRule
  isDeleting: boolean
  onDelete: (rule: SiteRule) => void
}

function SiteRuleRow({ rule, isDeleting, onDelete }: SiteRuleRowProps) {
  return (
    <div className="flex w-full items-center gap-3 rounded-xl border border-border-2 bg-card p-3.5">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate font-semibold text-ink text-sm">
            {rule.label}
          </span>
          <span
            className={cn(
              'shrink-0 rounded-full px-2 py-0.5 font-bold text-[10.5px] uppercase tracking-wider',
              siteRuleActionBadgeClass(rule.action),
            )}
          >
            {siteRuleActionLabel(rule.action)}
          </span>
        </div>
        <div className="mt-0.5 truncate font-mono text-ink-3 text-xs">
          {rule.domain}
        </div>
      </div>
      <Button
        type="button"
        variant="outline"
        size="sm"
        onClick={() => onDelete(rule)}
        disabled={isDeleting}
        aria-label={`Remove ${rule.label}`}
        className="shrink-0 px-2.5 text-ink-3 hover:text-red"
      >
        <Trash2 className="size-3.5" />
      </Button>
    </div>
  )
}
