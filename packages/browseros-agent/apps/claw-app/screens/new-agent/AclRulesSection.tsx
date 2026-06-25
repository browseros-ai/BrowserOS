import { Check, Plus, Trash2, X } from 'lucide-react'
import { type KeyboardEvent, useId, useState } from 'react'
import { useFormContext } from 'react-hook-form'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import {
  type AclRule,
  normalizeDomainInput,
  SEED_ACL_RULES,
} from './new-agent.helpers'
import type { NewAgentValues } from './new-agent.schemas'

export function AclRulesSection() {
  const form = useFormContext<NewAgentValues>()
  const aclRuleIds = form.watch('aclRuleIds')
  const customAclRules = form.watch('customAclRules')

  const [adding, setAdding] = useState(false)
  const [draftLabel, setDraftLabel] = useState('')
  const [draftDomain, setDraftDomain] = useState('')
  const labelFieldId = useId()
  const domainFieldId = useId()

  const allRules: AclRule[] = [...SEED_ACL_RULES, ...customAclRules]
  const allEnforced =
    allRules.length > 0 &&
    allRules.every((rule) => aclRuleIds.includes(rule.id))

  const toggleRule = (id: string) => {
    form.setValue(
      'aclRuleIds',
      aclRuleIds.includes(id)
        ? aclRuleIds.filter((r) => r !== id)
        : [...aclRuleIds, id],
      { shouldDirty: true },
    )
  }

  const selectAll = () => {
    form.setValue('aclRuleIds', allEnforced ? [] : allRules.map((r) => r.id), {
      shouldDirty: true,
    })
  }

  const resetDraft = () => {
    setDraftLabel('')
    setDraftDomain('')
    setAdding(false)
  }

  const draftValid =
    draftLabel.trim().length > 0 && draftDomain.trim().length > 0

  const commitDraft = () => {
    if (!draftValid) return
    const id = `custom-${Date.now()}`
    const rule: AclRule = {
      id,
      label: draftLabel.trim(),
      domain: normalizeDomainInput(draftDomain),
      custom: true,
    }
    form.setValue('customAclRules', [...customAclRules, rule], {
      shouldDirty: true,
    })
    form.setValue('aclRuleIds', [...aclRuleIds, id], { shouldDirty: true })
    resetDraft()
  }

  const removeCustom = (id: string) => {
    form.setValue(
      'customAclRules',
      customAclRules.filter((r) => r.id !== id),
      { shouldDirty: true },
    )
    form.setValue(
      'aclRuleIds',
      aclRuleIds.filter((r) => r !== id),
      { shouldDirty: true },
    )
  }

  const onDraftKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') {
      event.preventDefault()
      commitDraft()
    }
  }

  return (
    <div className="flex flex-col gap-2.5">
      <div className="flex items-center justify-between">
        <span className="text-ink-3 text-xs">
          {aclRuleIds.length} of {allRules.length} enforced
        </span>
        <Button
          type="button"
          variant="outline"
          size="sm"
          onClick={selectAll}
          className="h-7 gap-1.5 px-2.5 text-xs"
        >
          {allEnforced ? (
            <X className="size-3" />
          ) : (
            <Check className="size-3" />
          )}
          {allEnforced ? 'Clear all' : 'Select all'}
        </Button>
      </div>

      <div className="flex flex-col gap-2">
        {allRules.map((rule) => {
          const enforced = aclRuleIds.includes(rule.id)
          return (
            <div
              key={rule.id}
              className={cn(
                'flex w-full items-stretch rounded-xl border transition-colors',
                enforced
                  ? 'border-red-tint bg-red-tint'
                  : 'border-border-2 bg-card hover:border-border-strong',
              )}
            >
              <button
                type="button"
                onClick={() => toggleRule(rule.id)}
                className="flex flex-1 items-center gap-3 p-3.5 text-left"
              >
                <span
                  className={cn(
                    'flex size-[18px] shrink-0 items-center justify-center rounded-[5px] border-[1.5px]',
                    enforced
                      ? 'border-red bg-red text-card'
                      : 'border-border-strong bg-card',
                  )}
                >
                  {enforced && <Check className="size-3" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5">
                    <span className="font-semibold text-ink text-sm">
                      {rule.label}
                    </span>
                    {rule.custom && (
                      <Badge
                        variant="outline"
                        className="h-4 border-accent-tint-2 bg-accent-tint px-1.5 font-bold text-[9px] text-accent uppercase tracking-wider"
                      >
                        New
                      </Badge>
                    )}
                  </div>
                  <div className="font-mono text-ink-3 text-xs">
                    {rule.domain}
                  </div>
                </div>
              </button>
              {rule.custom && (
                <button
                  type="button"
                  onClick={() => removeCustom(rule.id)}
                  aria-label={`Remove ${rule.label}`}
                  className="flex shrink-0 items-center justify-center px-3 text-ink-3 hover:text-red"
                >
                  <Trash2 className="size-3.5" />
                </button>
              )}
            </div>
          )
        })}

        {adding ? (
          <div className="flex flex-col gap-2 rounded-xl border border-border-strong border-dashed bg-bg-sunken p-3">
            <Input
              id={labelFieldId}
              autoFocus
              value={draftLabel}
              onChange={(event) => setDraftLabel(event.target.value)}
              onKeyDown={onDraftKeyDown}
              placeholder="What to block — e.g. Wire transfers"
            />
            <div className="flex gap-2">
              <Input
                id={domainFieldId}
                value={draftDomain}
                onChange={(event) => setDraftDomain(event.target.value)}
                onKeyDown={onDraftKeyDown}
                placeholder="domain — e.g. mercury.com"
                className="font-mono"
              />
              <Button
                type="button"
                onClick={commitDraft}
                disabled={!draftValid}
                size="sm"
                className="shrink-0"
              >
                Add
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={resetDraft}
                size="sm"
                className="shrink-0"
              >
                Cancel
              </Button>
            </div>
          </div>
        ) : (
          <Button
            type="button"
            variant="outline"
            onClick={() => setAdding(true)}
            className="h-11 w-full gap-1.5 border-dashed text-ink-2"
          >
            <Plus className="size-3.5" />
            Add a rule
          </Button>
        )}
      </div>
    </div>
  )
}
