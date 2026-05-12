import type { FC } from 'react'
import { cn } from '@/lib/utils'

interface MethodSelectorProps {
  method: 'default' | 'vcc'
  onMethodChange: (method: 'default' | 'vcc') => void
}

const methods = [
  {
    value: 'default' as const,
    label: 'Default (LLM Summarization)',
    description:
      'Uses the AI model to summarize conversation history. Produces a structured summary with Goal, Progress, Next Steps.',
  },
  {
    value: 'vcc' as const,
    label: 'VCC (Algorithmic)',
    description:
      'Structured extraction without LLM calls. Faster (30–470ms vs 1–2s), zero token cost.',
  },
]

export const MethodSelector: FC<MethodSelectorProps> = ({
  method,
  onMethodChange,
}) => {
  return (
    <div className="space-y-3">
      <div className="space-y-2">
        {methods.map((m) => (
          <button
            key={m.value}
            type="button"
            onClick={() => onMethodChange(m.value)}
            className={cn(
              'w-full rounded-lg border p-4 text-left transition-colors',
              method === m.value
                ? 'border-primary bg-primary/5'
                : 'border-border hover:border-primary/50',
            )}
          >
            <div className="flex items-center gap-3">
              <div
                className={cn(
                  'flex size-5 shrink-0 items-center justify-center rounded-full border-2',
                  method === m.value
                    ? 'border-primary bg-primary'
                    : 'border-muted-foreground/30',
                )}
              >
                {method === m.value && (
                  <div className="size-2 rounded-full bg-primary-foreground" />
                )}
              </div>
              <div className="font-medium text-sm">{m.label}</div>
            </div>
            <p className="mt-2 pl-8 text-muted-foreground text-xs leading-relaxed">
              {m.description}
            </p>
          </button>
        ))}
      </div>
    </div>
  )
}
