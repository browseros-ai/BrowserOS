import { Bot, Code, MousePointer2, Sparkles, Terminal } from 'lucide-react'
import { type ComponentType, useId } from 'react'
import { useFormContext } from 'react-hook-form'
import {
  FormControl,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import { RadioGroup, RadioGroupItem } from '@/components/ui/radio-group'
import { cn } from '@/lib/utils'
import type { Harness, NewAgentValues } from './new-agent.schemas'

interface HarnessEntry {
  name: Harness
  Icon: ComponentType<{ className?: string }>
}

const HARNESS_ENTRIES: readonly HarnessEntry[] = [
  { name: 'Claude Cowork', Icon: Sparkles },
  { name: 'Codex', Icon: Code },
  { name: 'Hermes', Icon: Bot },
  { name: 'OpenClaw', Icon: MousePointer2 },
  { name: 'Gemini CLI', Icon: Terminal },
]

export function HarnessSection() {
  const form = useFormContext<NewAgentValues>()
  const nameId = useId()

  return (
    <div className="flex flex-col gap-4">
      <FormField
        control={form.control}
        name="name"
        render={({ field }) => (
          <FormItem>
            <FormLabel
              htmlFor={nameId}
              className="font-medium text-ink-2 text-xs"
            >
              Connector name
            </FormLabel>
            <FormControl>
              <Input
                id={nameId}
                placeholder="e.g. Codex · Finance"
                {...field}
              />
            </FormControl>
            <FormMessage />
          </FormItem>
        )}
      />

      <FormField
        control={form.control}
        name="harness"
        render={({ field }) => (
          <FormItem>
            <FormLabel className="font-medium text-ink-2 text-xs">
              Harness
            </FormLabel>
            <FormControl>
              <RadioGroup
                value={field.value}
                onValueChange={(value) => field.onChange(value as Harness)}
                className="flex flex-wrap gap-2"
              >
                {HARNESS_ENTRIES.map(({ name, Icon }) => {
                  const selected = field.value === name
                  return (
                    <label
                      key={name}
                      htmlFor={`harness-${name}`}
                      className={cn(
                        'flex cursor-pointer items-center gap-2 rounded-lg border px-3 py-2 transition-colors',
                        selected
                          ? 'border-accent bg-accent-tint text-accent-ink'
                          : 'border-border-2 bg-card text-ink-2 hover:border-border-strong',
                      )}
                    >
                      <RadioGroupItem
                        id={`harness-${name}`}
                        value={name}
                        className="sr-only"
                      />
                      <Icon className="size-4" />
                      <span className="font-semibold text-xs">{name}</span>
                    </label>
                  )
                })}
              </RadioGroup>
            </FormControl>
          </FormItem>
        )}
      />
    </div>
  )
}
