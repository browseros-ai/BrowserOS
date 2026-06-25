import { Check, Lock, X } from 'lucide-react'
import { useFormContext } from 'react-hook-form'
import { Button } from '@/components/ui/button'
import { FormControl, FormField, FormItem } from '@/components/ui/form'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { cn } from '@/lib/utils'
import { IMPORTED_SITES, TOTAL_PROFILE_LOGINS } from './new-agent.helpers'
import type { LoginMode, NewAgentValues } from './new-agent.schemas'

interface ModeOption {
  value: LoginMode
  title: string
  description: string
}

const MODES: readonly ModeOption[] = [
  {
    value: 'profile',
    title: 'All sites from current profile',
    description:
      'Inherit every login in the Chrome profile you imported. Simplest.',
  },
  {
    value: 'all',
    title: `All my logins (${TOTAL_PROFILE_LOGINS} sites)`,
    description: 'Every session across every imported profile.',
  },
  {
    value: 'selective',
    title: 'Selective — only specific sites',
    description:
      'The agent can only act where you grant a login. Most contained.',
  },
]

export function LoginsSection() {
  const form = useFormContext<NewAgentValues>()
  const loginMode = form.watch('loginMode')
  const selectedSites = form.watch('selectedSites')

  const allSitesOn = IMPORTED_SITES.every((site) =>
    selectedSites.includes(site),
  )

  const toggleSite = (site: string) => {
    form.setValue(
      'selectedSites',
      selectedSites.includes(site)
        ? selectedSites.filter((s) => s !== site)
        : [...selectedSites, site],
      { shouldDirty: true },
    )
  }

  const selectAllSites = () => {
    form.setValue('selectedSites', allSitesOn ? [] : [...IMPORTED_SITES], {
      shouldDirty: true,
    })
  }

  return (
    <div className="flex flex-col gap-3">
      <FormField
        control={form.control}
        name="loginMode"
        render={({ field }) => (
          <FormItem>
            <FormControl>
              <RadioGroup
                value={field.value}
                onValueChange={(value) => field.onChange(value as LoginMode)}
                className="flex flex-col gap-3"
              >
                {MODES.map((mode) => {
                  const selected = field.value === mode.value
                  return (
                    <label
                      key={mode.value}
                      htmlFor={`login-${mode.value}`}
                      className={cn(
                        'flex cursor-pointer items-start gap-3 rounded-xl border p-3.5 transition-colors',
                        selected
                          ? 'border-accent bg-accent-tint'
                          : 'border-border-2 bg-card hover:border-border-strong',
                      )}
                    >
                      <RadioGroupItem
                        id={`login-${mode.value}`}
                        value={mode.value}
                        className="mt-0.5"
                      />
                      <div className="flex flex-col gap-0.5">
                        <span className="font-semibold text-ink text-sm">
                          {mode.title}
                        </span>
                        <span className="text-ink-3 text-xs leading-snug">
                          {mode.description}
                        </span>
                      </div>
                    </label>
                  )
                })}
              </RadioGroup>
            </FormControl>
          </FormItem>
        )}
      />

      {loginMode === 'selective' && (
        <div className="flex flex-col gap-2 pt-1">
          <div className="flex items-center justify-between">
            <span className="text-ink-3 text-xs">
              {selectedSites.length} of {IMPORTED_SITES.length} sites
            </span>
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={selectAllSites}
              className="h-7 gap-1.5 px-2.5 text-xs"
            >
              {allSitesOn ? (
                <X className="size-3" />
              ) : (
                <Check className="size-3" />
              )}
              {allSitesOn ? 'Clear all' : 'Select all'}
            </Button>
          </div>
          <div className="flex flex-wrap gap-1.5">
            {IMPORTED_SITES.map((site) => {
              const selected = selectedSites.includes(site)
              return (
                <button
                  key={site}
                  type="button"
                  onClick={() => toggleSite(site)}
                  className={cn(
                    'flex items-center gap-1.5 rounded-lg border px-2.5 py-1.5 font-mono text-xs transition-colors',
                    selected
                      ? 'border-accent bg-accent-tint text-accent-ink'
                      : 'border-border-2 bg-card text-ink-2 hover:border-border-strong',
                  )}
                >
                  {selected ? (
                    <Check className="size-3" />
                  ) : (
                    <Lock className="size-3" />
                  )}
                  {site}
                </button>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}
