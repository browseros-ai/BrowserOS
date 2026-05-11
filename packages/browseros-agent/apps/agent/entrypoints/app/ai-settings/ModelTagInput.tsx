'use client'

import { X } from 'lucide-react'
import {
  type FC,
  type KeyboardEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from 'react'
import { cn } from '@/lib/utils'

/**
 * Props for ModelTagInput
 * @public
 */
export interface ModelTagInputProps {
  /** Current list of model IDs */
  models: string[]
  /** Callback when models change */
  onModelsChange: (models: string[]) => void
  /** Currently active model ID */
  activeModel: string
  /** Callback when first model is added (to auto-set active) */
  onActiveModelChange: (modelId: string) => void
  /** Autocomplete suggestions */
  suggestions?: string[]
  /** Whether the input is disabled */
  disabled?: boolean
}

/** Duration of the flash/shake animation in ms */
const FLASH_DURATION = 600

/**
 * Tag/chip input for model names.
 *
 * - SPACE or ENTER commits the current input as a chip
 * - SPACE also splits on whitespace: "gpt-4o gpt-4o-mini" → 2 chips
 * - BACKSPACE on empty input removes the last chip
 * - Duplicate entries flash the existing chip without adding
 * - First model added when activeModel is empty → calls onActiveModelChange
 */
export const ModelTagInput: FC<ModelTagInputProps> = ({
  models,
  onModelsChange,
  activeModel,
  onActiveModelChange,
  suggestions: _suggestions,
  disabled = false,
}) => {
  const [inputValue, setInputValue] = useState('')
  const [flashingId, setFlashingId] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const flashTimerRef = useRef<ReturnType<typeof setTimeout>>(null)

  // Cleanup flash timer on unmount
  useEffect(() => {
    return () => {
      if (flashTimerRef.current) clearTimeout(flashTimerRef.current)
    }
  }, [])

  const flashChip = useCallback((modelId: string) => {
    setFlashingId(modelId)
    if (flashTimerRef.current) clearTimeout(flashTimerRef.current)
    flashTimerRef.current = setTimeout(
      () => setFlashingId(null),
      FLASH_DURATION,
    )
  }, [])

  const addModels = useCallback(
    (raw: string) => {
      // Split on whitespace, trim, skip empty
      const candidates = raw
        .split(/\s+/)
        .map((s) => s.trim())
        .filter(Boolean)

      if (candidates.length === 0) return

      let changed = false
      const next = [...models]

      for (const id of candidates) {
        if (next.includes(id)) {
          // Duplicate — flash existing chip
          flashChip(id)
          continue
        }
        next.push(id)
        changed = true
      }

      if (changed) {
        onModelsChange(next)
        // If activeModel was empty and we added at least one new model, auto-set
        const firstNewModel = candidates.find((id) => !models.includes(id))
        if (!activeModel && firstNewModel) {
          onActiveModelChange(firstNewModel)
        }
      }

      setInputValue('')
    },
    [models, onModelsChange, activeModel, onActiveModelChange, flashChip],
  )

  const removeModel = useCallback(
    (modelId: string) => {
      const next = models.filter((m) => m !== modelId)
      onModelsChange(next)
    },
    [models, onModelsChange],
  )

  const handleKeyDown = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (disabled) return

      if (e.key === 'Enter') {
        e.preventDefault()
        addModels(inputValue)
        return
      }

      if (e.key === ' ') {
        e.preventDefault()
        addModels(inputValue)
        return
      }

      if (e.key === 'Backspace' && inputValue === '' && models.length > 0) {
        e.preventDefault()
        removeModel(models[models.length - 1])
        return
      }

      if (e.key === 'Escape') {
        setInputValue('')
        inputRef.current?.blur()
      }
    },
    [disabled, inputValue, models, addModels, removeModel],
  )

  const handleContainerClick = () => {
    if (!disabled) inputRef.current?.focus()
  }

  return (
    // biome-ignore lint/a11y/useSemanticElements: container div needs flex layout, not a <button>
    <div
      className={cn(
        'flex min-h-9 cursor-text flex-wrap items-center gap-1.5 rounded-md border border-input bg-transparent px-3 py-1.5 text-sm shadow-xs',
        disabled && 'cursor-not-allowed opacity-50',
      )}
      onClick={handleContainerClick}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') handleContainerClick()
      }}
      role="button"
      tabIndex={0}
      aria-label="Model tags"
    >
      {models.map((modelId) => (
        <span
          key={modelId}
          className={cn(
            'inline-flex items-center gap-1 rounded-md bg-muted px-2 py-0.5 font-medium text-foreground text-xs transition-all',
            flashingId === modelId &&
              'animate-shake bg-destructive/15 text-destructive',
            modelId === activeModel && 'bg-primary/10 ring-1 ring-primary/40',
          )}
        >
          {modelId}
          {!disabled && (
            <button
              type="button"
              className="ml-0.5 inline-flex h-3.5 w-3.5 items-center justify-center rounded-sm hover:bg-muted-foreground/20"
              onClick={(e) => {
                e.stopPropagation()
                removeModel(modelId)
              }}
              aria-label={`Remove ${modelId}`}
            >
              <X className="h-2.5 w-2.5" />
            </button>
          )}
        </span>
      ))}

      <input
        ref={inputRef}
        type="text"
        value={inputValue}
        onChange={(e) => setInputValue(e.target.value)}
        onKeyDown={handleKeyDown}
        disabled={disabled}
        placeholder={
          models.length === 0
            ? 'Add models to get started…'
            : 'Type to add model, SPACE to confirm…'
        }
        className="min-w-[120px] flex-1 bg-transparent outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
      />
    </div>
  )
}
