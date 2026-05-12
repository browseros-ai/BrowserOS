import { Layers } from 'lucide-react'
import type { FC } from 'react'
import { useEffect, useMemo, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { PendingRestartBanner } from '../advanced-config/PendingRestartBanner'
import {
  type CompactionConfig,
  useCompactionConfig,
  type VccConfig,
} from './compaction-queries'
import { MethodSelector } from './MethodSelector'
import {
  VCC_DEFAULTS,
  VCC_FIELDS,
  VccConfigSection,
  validateVccField,
} from './VccConfigSection'

// Single source of truth: VCC defaults come from VccConfigSection's VCC_FIELDS
const VCC_FIELD_DEFAULTS = VCC_DEFAULTS

export const CompactionSettingsPage: FC = () => {
  const {
    config,
    isLoading,
    error,
    saveConfig,
    resetConfig,
    isSaving,
    isResetting,
  } = useCompactionConfig()

  const [method, setMethod] = useState<'default' | 'vcc'>('default')
  const [customPrompt, setCustomPrompt] = useState('')
  const [vccConfig, setVccConfig] = useState<VccConfig>({})
  const [hasPendingRestart, setHasPendingRestart] = useState(false)

  // Populate form from server data on first load only
  useEffect(() => {
    if (!config) return
    const active = config.active
    if (active) {
      setMethod(active.method ?? 'default')
      setCustomPrompt(active.customPrompt ?? '')
      setVccConfig(active.vccConfig ?? {})
    } else {
      setMethod(config.defaults.method)
      setCustomPrompt('')
      setVccConfig({})
    }
    // NOTE: Do NOT reset hasPendingRestart here.
    // After save+invalidate, the refetch triggers this effect.
    // Resetting would hide the banner after one frame.
  }, [config])

  // Validate VCC fields
  const vccErrors = useMemo(() => {
    const errs: Record<string, string> = {}
    for (const field of VCC_FIELDS) {
      const raw = String(vccConfig[field.key] ?? VCC_FIELD_DEFAULTS[field.key])
      const err = validateVccField(field, raw)
      if (err) errs[field.key] = err
    }
    return errs
  }, [vccConfig])

  const hasValidationErrors =
    method === 'vcc' && Object.keys(vccErrors).length > 0

  const hasChanges = useMemo(() => {
    if (!config) return false
    const active = config.active

    const currentMethod = active?.method ?? 'default'
    if (method !== currentMethod) return true

    if (method === 'default') {
      const currentPrompt = active?.customPrompt ?? ''
      if (customPrompt !== currentPrompt) return true
    }

    if (method === 'vcc') {
      const currentVcc = active?.vccConfig ?? {}
      if (JSON.stringify(vccConfig) !== JSON.stringify(currentVcc)) return true
    }

    return false
  }, [config, method, customPrompt, vccConfig])

  const handleSave = async () => {
    if (hasValidationErrors) {
      toast.error('Fix validation errors before saving')
      return
    }

    const newConfig: CompactionConfig = { method }

    if (method === 'default' && customPrompt.trim()) {
      newConfig.customPrompt = customPrompt.trim()
    }

    if (method === 'vcc') {
      const overrides: VccConfig = {}
      for (const [key, defaultVal] of Object.entries(VCC_FIELD_DEFAULTS)) {
        const current = vccConfig[key as keyof VccConfig]
        if (current !== undefined && current !== defaultVal) {
          ;(overrides as Record<string, number>)[key] = current
        }
      }
      if (Object.keys(overrides).length > 0) {
        newConfig.vccConfig = overrides
      }
    }

    try {
      const result = await saveConfig(newConfig)
      if (!result.ok) {
        toast.error(
          result.errors?.[0]?.message ?? 'Failed to save compaction config',
        )
        return
      }
    } catch {
      toast.error('Failed to save compaction config')
      return
    }

    setHasPendingRestart(true)
    toast.success(
      'Compaction config saved. Quit and reopen BrowserOS to apply.',
    )
  }

  const handleReset = async () => {
    try {
      const result = await resetConfig()
      if (!result.ok) {
        toast.error(
          result.errors?.[0]?.message ?? 'Failed to reset compaction config',
        )
        return
      }
    } catch {
      toast.error('Failed to reset compaction config')
      return
    }

    setHasPendingRestart(false)
    toast.success(
      'Compaction config reset. Quit and reopen BrowserOS to apply.',
    )
  }

  if (isLoading) {
    return (
      <div className="flex items-center justify-center p-12 text-muted-foreground text-sm">
        Loading compaction settings...
      </div>
    )
  }

  if (error || !config) {
    return (
      <div className="rounded-xl border border-destructive/30 bg-destructive/5 p-6 text-destructive">
        Failed to load compaction settings.
      </div>
    )
  }

  return (
    <div className="space-y-6 p-6">
      <div className="flex items-start gap-4 rounded-xl border p-5">
        <div className="rounded-lg bg-muted p-2">
          <Layers className="size-6" />
        </div>
        <div>
          <h2 className="font-semibold text-lg">Compaction</h2>
          <p className="text-muted-foreground text-sm">
            Configure how conversation history is compressed when the context
            window fills up. Changes require restarting BrowserOS.
          </p>
        </div>
      </div>

      <PendingRestartBanner visible={hasPendingRestart} />

      <div className="space-y-4">
        <div>
          <h3 className="mb-3 font-medium text-sm">Method</h3>
          <MethodSelector method={method} onMethodChange={setMethod} />
        </div>

        {method === 'default' && (
          <div>
            <h3 className="mb-2 font-medium text-sm">
              Custom Prompt{' '}
              <span className="font-normal text-muted-foreground">
                (optional)
              </span>
            </h3>
            <p className="mb-2 text-muted-foreground text-xs">
              Override the default summarization prompt used by the LLM.
            </p>
            <textarea
              value={customPrompt}
              onChange={(e) => setCustomPrompt(e.target.value)}
              placeholder="Override the default summarization prompt..."
              rows={4}
              className="w-full resize-y rounded-lg border bg-transparent p-3 font-mono text-sm placeholder:text-muted-foreground/50 focus:outline-none focus:ring-2 focus:ring-primary/20"
            />
          </div>
        )}

        {method === 'vcc' && (
          <VccConfigSection
            values={vccConfig}
            onChange={setVccConfig}
            errors={vccErrors}
          />
        )}
      </div>

      <div className="flex flex-col gap-3 sm:flex-row sm:justify-between">
        <Button
          variant="outline"
          onClick={() => void handleReset()}
          disabled={isSaving || isResetting}
        >
          Reset to Defaults
        </Button>
        <Button
          onClick={() => void handleSave()}
          disabled={
            isSaving || isResetting || hasValidationErrors || !hasChanges
          }
        >
          {isSaving ? 'Saving...' : 'Save Changes'}
        </Button>
      </div>
    </div>
  )
}
