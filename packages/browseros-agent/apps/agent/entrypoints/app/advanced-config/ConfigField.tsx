import { RotateCcw } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import type { ConfigKeySchema } from './config-queries'

interface ConfigFieldProps {
  schema: ConfigKeySchema
  value: string
  currentValue: number
  defaultValue: number
  error?: string
  onValueChange: (key: string, value: string) => void
  onReset: (key: string) => void
}

export const ConfigField: FC<ConfigFieldProps> = ({
  schema,
  value,
  currentValue,
  defaultValue,
  error,
  onValueChange,
  onReset,
}) => {
  const modified = currentValue !== defaultValue

  return (
    <div className="flex flex-col gap-3 py-4 md:flex-row md:items-start md:justify-between">
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex items-center gap-2">
          <Label className="font-medium text-sm">{schema.label}</Label>
          {modified ? (
            <span className="rounded-full bg-amber-500/10 px-2 py-0.5 text-[10px] text-amber-700 uppercase tracking-wide dark:text-amber-300">
              modified
            </span>
          ) : null}
        </div>
        <p className="text-muted-foreground text-xs">{schema.description}</p>
        <p className="text-[11px] text-muted-foreground">
          Default: {defaultValue.toLocaleString()} {schema.unit} • ENV:{' '}
          <code>{schema.envVar}</code>
        </p>
        {schema.risk ? (
          <p className="text-amber-700 text-xs dark:text-amber-300">
            ⚠ {schema.risk}
          </p>
        ) : null}
        {error ? <p className="text-destructive text-xs">{error}</p> : null}
      </div>

      <div className="flex items-center gap-2 md:shrink-0">
        <div className="relative">
          <Input
            value={value}
            inputMode="numeric"
            onChange={(e) => onValueChange(schema.key, e.target.value)}
            className="w-40 pr-12 text-right"
            aria-invalid={!!error}
          />
          <span className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-muted-foreground text-xs">
            {schema.unit}
          </span>
        </div>
        <Button
          variant="ghost"
          size="icon"
          onClick={() => onReset(schema.key)}
          title="Reset to default"
        >
          <RotateCcw className="size-4" />
        </Button>
      </div>
    </div>
  )
}
