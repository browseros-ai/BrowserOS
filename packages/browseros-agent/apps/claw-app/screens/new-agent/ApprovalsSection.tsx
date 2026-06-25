import { useFormContext } from 'react-hook-form'
import { FormControl, FormField, FormItem } from '@/components/ui/form'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { cn } from '@/lib/utils'
import {
  APPROVAL_CATEGORIES,
  type ApprovalCategory,
  type ApprovalVerdict,
  type NewAgentValues,
} from './new-agent.schemas'

const VERDICT_COLORS: Record<ApprovalVerdict, string> = {
  Auto: 'aria-pressed:text-green aria-pressed:bg-green-tint',
  Ask: 'aria-pressed:text-amber aria-pressed:bg-amber-tint',
  Block: 'aria-pressed:text-red aria-pressed:bg-red-tint',
}

function availableVerdicts(category: ApprovalCategory): ApprovalVerdict[] {
  return category.allowAuto ? ['Auto', 'Ask', 'Block'] : ['Ask', 'Block']
}

export function ApprovalsSection() {
  const form = useFormContext<NewAgentValues>()

  return (
    <div className="overflow-hidden rounded-xl border border-border-2 bg-card">
      {APPROVAL_CATEGORIES.map((category, idx) => (
        <FormField
          key={category.id}
          control={form.control}
          name={`approvals.${category.id}`}
          render={({ field }) => (
            <FormItem
              className={cn(
                'flex flex-row items-center gap-3 px-4 py-3',
                idx < APPROVAL_CATEGORIES.length - 1 &&
                  'border-border border-b',
              )}
            >
              <span className="flex-1 font-semibold text-ink text-sm">
                {category.name}
              </span>
              <FormControl>
                <ToggleGroup
                  value={field.value ? [field.value] : []}
                  onValueChange={(values) => {
                    const next = values[0]
                    if (next) field.onChange(next as ApprovalVerdict)
                  }}
                  spacing={0}
                  variant="outline"
                  className="bg-bg-sunken p-0.5"
                >
                  {availableVerdicts(category).map((verdict) => (
                    <ToggleGroupItem
                      key={verdict}
                      value={verdict}
                      className={cn(
                        'h-7 rounded-md border-none px-2.5 font-bold text-[11px] text-ink-3 uppercase tracking-wide',
                        VERDICT_COLORS[verdict],
                      )}
                    >
                      {verdict}
                    </ToggleGroupItem>
                  ))}
                </ToggleGroup>
              </FormControl>
            </FormItem>
          )}
        />
      ))}
    </div>
  )
}
