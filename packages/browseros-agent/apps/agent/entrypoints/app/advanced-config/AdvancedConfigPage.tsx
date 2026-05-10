import { Settings2 } from 'lucide-react'
import type { FC } from 'react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { ConfigGroup } from './ConfigGroup'
import { type ConfigKeySchema, useAdvancedConfig } from './config-queries'
import { DangerousSection } from './DangerousSection'
import { PendingRestartBanner } from './PendingRestartBanner'

type GroupName = 'Timeouts' | 'Limits' | 'Retention'

const GROUP_ORDER: GroupName[] = ['Timeouts', 'Limits', 'Retention']

function normalizeNumericInput(raw: string): string {
  return raw.trim().replace(/_/g, '')
}

function validateField(schema: ConfigKeySchema, raw: string): string | null {
  const normalized = normalizeNumericInput(raw)
  if (!/^\d+$/.test(normalized)) return 'Enter a whole non-negative integer'

  const value = Number(normalized)
  if (!Number.isSafeInteger(value)) return 'Value must be a safe integer'
  if (value < schema.min) return `Minimum is ${schema.min.toLocaleString()}`
  if (value > schema.max) return `Maximum is ${schema.max.toLocaleString()}`
  return null
}

export const AdvancedConfigPage: FC = () => {
  const {
    config,
    isLoading,
    error,
    saveConfig,
    resetConfig,
    isSaving,
    isResetting,
  } = useAdvancedConfig()

  const [draftValues, setDraftValues] = useState<Record<string, string>>({})
  const [groupOpen, setGroupOpen] = useState<Record<GroupName, boolean>>({
    Timeouts: true,
    Limits: true,
    Retention: true,
  })
  const [dangerousOpen, setDangerousOpen] = useState(false)

  useEffect(() => {
    if (!config) return
    const nextDraft: Record<string, string> = {}
    for (const [key, value] of Object.entries(config.pending)) {
      nextDraft[key] = String(value)
    }
    for (const [key, value] of Object.entries(config.active)) {
      if (!(key in nextDraft)) nextDraft[key] = String(value)
    }
    setDraftValues(nextDraft)
  }, [config])

  const fieldEntries = useMemo(() => {
    if (!config) return []
    return Object.values(config.schema).sort((a, b) =>
      a.label.localeCompare(b.label),
    )
  }, [config])

  const errors = useMemo(() => {
    if (!config) return {} as Record<string, string>
    const next: Record<string, string> = {}
    for (const field of fieldEntries) {
      const raw =
        draftValues[field.key] ??
        String(
          config.pending[field.key] ??
            config.active[field.key] ??
            field.default,
        )
      const error = validateField(field, raw)
      if (error) next[field.key] = error
    }
    return next
  }, [config, draftValues, fieldEntries])

  const parsedValues = useMemo(() => {
    if (!config) return {} as Record<string, number>
    const next: Record<string, number> = {}
    for (const field of fieldEntries) {
      const raw =
        draftValues[field.key] ??
        String(
          config.pending[field.key] ??
            config.active[field.key] ??
            field.default,
        )
      const normalized = normalizeNumericInput(raw)
      next[field.key] = /^\d+$/.test(normalized)
        ? Number(normalized)
        : (config.pending[field.key] ??
          config.active[field.key] ??
          field.default)
    }
    return next
  }, [config, draftValues, fieldEntries])

  const fieldsByGroup = useMemo(() => {
    const groups: Record<GroupName, ConfigKeySchema[]> = {
      Timeouts: [],
      Limits: [],
      Retention: [],
    }
    const dangerous: ConfigKeySchema[] = []

    for (const field of fieldEntries) {
      if (field.section === 'dangerous') {
        dangerous.push(field)
      } else {
        groups[field.group].push(field)
      }
    }

    return { groups, dangerous }
  }, [fieldEntries])

  const hasValidationErrors = Object.keys(errors).length > 0
  const hasUnsavedChanges = useMemo(() => {
    if (!config) return false
    return fieldEntries.some((field) => {
      const pending =
        config.pending[field.key] ?? config.active[field.key] ?? field.default
      return parsedValues[field.key] !== pending
    })
  }, [config, fieldEntries, parsedValues])

  if (isLoading) {
    return (
      <div className="flex items-center justify-center p-12 text-muted-foreground text-sm">
        Loading advanced config...
      </div>
    )
  }

  if (error || !config) {
    return (
      <div className="rounded-xl border border-destructive/30 bg-destructive/5 p-6 text-destructive">
        Failed to load advanced config.
      </div>
    )
  }

  const handleValueChange = (key: string, value: string) => {
    setDraftValues((current) => ({ ...current, [key]: value }))
  }

  const handleResetField = (key: string) => {
    const nextValue = config.defaults[key] ?? config.active[key]
    setDraftValues((current) => ({ ...current, [key]: String(nextValue) }))
  }

  const handleSave = async () => {
    if (hasValidationErrors) {
      toast.error('Fix validation errors before saving')
      return
    }

    const overrides: Record<string, number> = {}
    for (const field of fieldEntries) {
      const value = parsedValues[field.key]
      if (value !== config.defaults[field.key]) {
        overrides[field.key] = value
      }
    }

    const result = await saveConfig(overrides)
    if (!result.ok) {
      toast.error(result.errors?.[0]?.message ?? 'Failed to save config')
      return
    }

    toast.success('Advanced config saved. Quit and reopen BrowserOS to apply.')
  }

  const handleResetAll = async () => {
    const result = await resetConfig()
    if (!result.ok) {
      toast.error(result.errors?.[0]?.message ?? 'Failed to reset config')
      return
    }

    toast.success('Advanced config reset. Quit and reopen BrowserOS to apply.')
  }

  return (
    <div className="space-y-6 p-6">
      <div className="flex items-start gap-4 rounded-xl border p-5">
        <div className="rounded-lg bg-muted p-2">
          <Settings2 className="size-6" />
        </div>
        <div>
          <h2 className="font-semibold text-lg">Advanced Config</h2>
          <p className="text-muted-foreground text-sm">
            Internal BrowserOS runtime settings. Changes are schema-validated
            and require restart.
          </p>
        </div>
      </div>

      <PendingRestartBanner visible={config.hasPendingChanges} />

      <div className="space-y-4">
        {GROUP_ORDER.map((group) => (
          <ConfigGroup
            key={group}
            title={group}
            open={groupOpen[group]}
            onToggle={() =>
              setGroupOpen((current) => ({
                ...current,
                [group]: !current[group],
              }))
            }
            fields={fieldsByGroup.groups[group]}
            values={draftValues}
            currentValues={parsedValues}
            defaults={config.defaults}
            errors={errors}
            onValueChange={handleValueChange}
            onReset={handleResetField}
          />
        ))}

        <DangerousSection
          open={dangerousOpen}
          onToggle={() => setDangerousOpen((current) => !current)}
          fields={fieldsByGroup.dangerous}
          values={draftValues}
          currentValues={parsedValues}
          defaults={config.defaults}
          errors={errors}
          onValueChange={handleValueChange}
          onReset={handleResetField}
        />
      </div>

      <div className="flex flex-col gap-3 sm:flex-row sm:justify-between">
        <Button
          variant="outline"
          onClick={() => void handleResetAll()}
          disabled={isSaving || isResetting}
        >
          Reset All to Defaults
        </Button>
        <Button
          onClick={() => void handleSave()}
          disabled={
            isSaving || isResetting || hasValidationErrors || !hasUnsavedChanges
          }
        >
          {isSaving ? 'Saving...' : 'Save Changes'}
        </Button>
      </div>
    </div>
  )
}
