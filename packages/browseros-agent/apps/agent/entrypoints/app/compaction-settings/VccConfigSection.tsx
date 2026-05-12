import { ChevronDown } from 'lucide-react'
import type { FC } from 'react'
import { useState } from 'react'
import { cn } from '@/lib/utils'
import type { VccConfig } from './compaction-queries'

interface VccConfigSectionProps {
  values: VccConfig
  onChange: (values: VccConfig) => void
}

interface FieldDef {
  key: keyof VccConfig
  label: string
  unit: string
  default: number
  min: number
  max: number
}

const VCC_FIELDS: FieldDef[] = [
  {
    key: 'maxTranscriptLines',
    label: 'Max Transcript Lines',
    unit: 'lines',
    default: 120,
    min: 10,
    max: 500,
  },
  {
    key: 'maxGoalLines',
    label: 'Max Goal Lines',
    unit: 'lines',
    default: 8,
    min: 1,
    max: 50,
  },
  {
    key: 'maxFileEntries',
    label: 'Max File Entries',
    unit: 'entries',
    default: 10,
    min: 1,
    max: 100,
  },
  {
    key: 'maxCommitEntries',
    label: 'Max Commit Entries',
    unit: 'entries',
    default: 8,
    min: 1,
    max: 50,
  },
  {
    key: 'maxPreferenceLines',
    label: 'Max Preference Lines',
    unit: 'lines',
    default: 15,
    min: 1,
    max: 100,
  },
  {
    key: 'maxOutstandingLines',
    label: 'Max Outstanding Lines',
    unit: 'lines',
    default: 10,
    min: 1,
    max: 100,
  },
]

export const VccConfigSection: FC<VccConfigSectionProps> = ({
  values,
  onChange,
}) => {
  const [open, setOpen] = useState(true)

  const handleFieldChange = (key: keyof VccConfig, raw: string) => {
    const num = Number.parseInt(raw, 10)
    if (Number.isNaN(num)) {
      const next = { ...values }
      delete next[key]
      onChange(next)
      return
    }
    onChange({ ...values, [key]: num })
  }

  return (
    <div className="rounded-lg border">
      <button
        type="button"
        onClick={() => setOpen((prev) => !prev)}
        className="flex w-full items-center justify-between p-4 text-left"
      >
        <span className="font-medium text-sm">VCC Configuration</span>
        <ChevronDown
          className={cn(
            'size-4 text-muted-foreground transition-transform',
            open && 'rotate-180',
          )}
        />
      </button>

      {open && (
        <div className="space-y-4 border-t p-4 pt-4">
          {VCC_FIELDS.map((field) => {
            const current = values[field.key]
            const isDefault = current === undefined || current === field.default

            return (
              <div key={field.key} className="flex items-center gap-4">
                <div className="min-w-0 flex-1">
                  <div className="font-medium text-sm">{field.label}</div>
                  <div className="text-muted-foreground text-xs">
                    Default: {field.default} {field.unit}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <input
                    type="number"
                    min={field.min}
                    max={field.max}
                    value={current ?? field.default}
                    onChange={(e) =>
                      handleFieldChange(field.key, e.target.value)
                    }
                    className={cn(
                      'h-9 w-24 rounded-md border bg-transparent px-3 text-right font-mono text-sm',
                      isDefault
                        ? 'border-border'
                        : 'border-primary/50 bg-primary/5',
                    )}
                  />
                  <span className="w-16 text-muted-foreground text-xs">
                    {field.unit}
                  </span>
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
