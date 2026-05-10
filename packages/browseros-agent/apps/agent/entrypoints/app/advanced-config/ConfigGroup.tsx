import { ChevronDown, ChevronRight } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import { ConfigField } from './ConfigField'
import type { ConfigKeySchema } from './config-queries'

interface ConfigGroupProps {
  title: string
  open: boolean
  onToggle: () => void
  fields: ConfigKeySchema[]
  values: Record<string, string>
  currentValues: Record<string, number>
  defaults: Record<string, number>
  errors: Record<string, string>
  onValueChange: (key: string, value: string) => void
  onReset: (key: string) => void
}

export const ConfigGroup: FC<ConfigGroupProps> = ({
  title,
  open,
  onToggle,
  fields,
  values,
  currentValues,
  defaults,
  errors,
  onValueChange,
  onReset,
}) => {
  return (
    <section className="overflow-hidden rounded-xl border bg-card">
      <Button
        variant="ghost"
        className="flex h-auto w-full items-center justify-start gap-2 rounded-none px-4 py-4 font-semibold text-base"
        onClick={onToggle}
      >
        {open ? (
          <ChevronDown className="size-4" />
        ) : (
          <ChevronRight className="size-4" />
        )}
        {title}
      </Button>

      {open ? (
        <div className="divide-y px-4">
          {fields.map((field) => (
            <ConfigField
              key={field.key}
              schema={field}
              value={
                values[field.key] ??
                String(currentValues[field.key] ?? defaults[field.key])
              }
              currentValue={currentValues[field.key] ?? defaults[field.key]}
              defaultValue={defaults[field.key]}
              error={errors[field.key]}
              onValueChange={onValueChange}
              onReset={onReset}
            />
          ))}
        </div>
      ) : null}
    </section>
  )
}
