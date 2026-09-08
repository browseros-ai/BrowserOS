import { Loader2, Pencil, X } from 'lucide-react'
import { Fragment, useEffect, useId, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import {
  STARTER_PROMPT_LABEL_LIMIT,
  STARTER_PROMPT_SLOTS,
  STARTER_PROMPT_TEXT_LIMIT,
  type StarterPrompt,
  starterPromptsSchema,
} from '@/modules/chat/starter-prompts'

interface StarterPromptEditorProps {
  prompts: StarterPrompt[]
  onSave: (prompts: StarterPrompt[]) => Promise<void>
  onCancel: () => void
}

/**
 * Owns one editing session's draft, independent of live storage updates.
 * Changing expanded rows never writes or runs a prompt; only Save commits.
 */
export function StarterPromptEditor({
  prompts,
  onSave,
  onCancel,
}: StarterPromptEditorProps) {
  const [draft, setDraft] = useState(() =>
    prompts.map((prompt) => ({ ...prompt })),
  )
  const [expanded, setExpanded] = useState(0)
  const [isSaving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const labelRef = useRef<HTMLInputElement>(null)
  const id = useId()

  // biome-ignore lint/correctness/useExhaustiveDependencies: each expanded row mounts a new label input that needs focus.
  useEffect(() => {
    labelRef.current?.focus()
  }, [expanded])

  const update = (slot: number, field: keyof StarterPrompt, value: string) => {
    setError(null)
    setDraft((current) =>
      current.map((prompt, index) =>
        index === slot ? { ...prompt, [field]: value } : prompt,
      ),
    )
  }

  const save = async () => {
    if (isSaving) return
    const result = starterPromptsSchema.safeParse(draft)
    if (!result.success) {
      const issue = result.error.issues[0]
      const slot = typeof issue.path[0] === 'number' ? issue.path[0] : 0
      setExpanded(slot)
      setError(`Prompt ${slot + 1}: ${issue.message}`)
      return
    }
    setSaving(true)
    setError(null)
    try {
      await onSave(result.data)
    } catch {
      // Failed persistence leaves the draft intact so the user can retry.
      setError('Could not save your prompts. Please try again.')
      setSaving(false)
    }
  }

  return (
    <form
      aria-label="Customize prompts"
      className="w-full max-w-[352px] text-left"
      noValidate
      onSubmit={(event) => {
        event.preventDefault()
        void save()
      }}
      onKeyDown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault()
          event.stopPropagation()
          if (!isSaving) onCancel()
        }
      }}
    >
      <fieldset disabled={isSaving} className="flex min-w-0 flex-col gap-2">
        {STARTER_PROMPT_SLOTS.map((slot) => {
          const prompt = draft[slot]
          const isExpanded = slot === expanded
          const regionId = `${id}-editor-${slot}`
          return (
            <Fragment key={slot}>
              <button
                type="button"
                aria-label={`Edit prompt ${slot + 1}: ${prompt.display || 'Untitled'}`}
                aria-expanded={isExpanded}
                aria-controls={isExpanded ? regionId : undefined}
                onClick={() => setExpanded(slot)}
                className={cn(
                  'flex min-h-11 items-center justify-between gap-3 rounded-lg border bg-card px-3 py-2.5 text-left text-[13px] disabled:opacity-60',
                  'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
                  isExpanded
                    ? 'border-[var(--accent-orange)]'
                    : 'border-border/50 hover:border-[var(--accent-orange)]/50',
                )}
              >
                <span className="min-w-0 flex-1 break-words">
                  {prompt.display || `Prompt ${slot + 1}`}
                </span>
                <Pencil
                  aria-hidden="true"
                  className={cn(
                    'size-3.5 shrink-0 text-muted-foreground',
                    isExpanded && 'text-[var(--accent-orange)]',
                  )}
                />
              </button>
              {isExpanded && (
                <div
                  id={regionId}
                  className="flex flex-col gap-3 rounded-xl border border-border bg-card p-4"
                >
                  <div className="flex items-center justify-between">
                    <h3 className="font-semibold text-sm">Edit prompt</h3>
                    <button
                      type="button"
                      aria-label="Cancel prompt customization"
                      onClick={onCancel}
                      className="rounded p-1 text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <X aria-hidden="true" className="size-4" />
                    </button>
                  </div>
                  <div className="flex flex-col gap-2">
                    <Label htmlFor={`${id}-label`} className="text-xs">
                      Button label
                    </Label>
                    <Input
                      ref={labelRef}
                      id={`${id}-label`}
                      value={prompt.display}
                      maxLength={STARTER_PROMPT_LABEL_LIMIT}
                      onChange={(event) =>
                        update(slot, 'display', event.target.value)
                      }
                      className="text-[13px] md:text-[13px]"
                    />
                  </div>
                  <div className="flex flex-col gap-2">
                    <Label htmlFor={`${id}-prompt`} className="text-xs">
                      Prompt
                    </Label>
                    <Textarea
                      id={`${id}-prompt`}
                      value={prompt.prompt}
                      maxLength={STARTER_PROMPT_TEXT_LIMIT}
                      onChange={(event) =>
                        update(slot, 'prompt', event.target.value)
                      }
                      aria-describedby={`${id}-help`}
                      className="h-28 max-h-60 min-h-24 resize-y text-[13px] leading-5 [field-sizing:fixed] md:text-[13px]"
                    />
                  </div>
                  <p
                    id={`${id}-help`}
                    className="text-muted-foreground text-xs"
                  >
                    The full prompt runs when you click the shortcut.
                  </p>
                </div>
              )}
            </Fragment>
          )
        })}
        {error && (
          <p role="alert" className="text-destructive text-xs">
            {error}
          </p>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <Button type="button" variant="outline" size="sm" onClick={onCancel}>
            Cancel
          </Button>
          <Button type="submit" size="sm">
            {isSaving && (
              <Loader2 aria-hidden="true" className="size-3.5 animate-spin" />
            )}
            {isSaving ? 'Saving…' : 'Save prompts'}
          </Button>
        </div>
      </fieldset>
    </form>
  )
}
