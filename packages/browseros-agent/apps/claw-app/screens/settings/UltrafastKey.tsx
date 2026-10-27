import type { AgentSettings } from '@browseros/claw-api'
import { JevTokenSource } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { useRef, useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { Input } from '@/components/ui/input'
import {
  useAgentSettings,
  useRemoveJevToken,
  useSaveJevToken,
} from '@/modules/api/agent-settings.hooks'

const KEY_INPUT_ID = 'jev-token-input'
const KEY_TITLE_ID = 'jev-token-title'
const KEY_DESCRIPTION_ID = 'jev-token-description'

/**
 * The Ultrafast access key, the one thing that decides whether agents are
 * offered Ultrafast at all. Three states rather than two: a key saved here, a
 * key BrowserOS already has from its own setup, or neither. Naming the second
 * case is what stops someone pasting a key they did not need.
 */
export function UltrafastKey({ source }: { source: JevTokenSource }) {
  // Uncontrolled on purpose: the key stays in the field it was typed into and
  // is read once on submit, never mirrored into React state.
  const field = useRef<HTMLInputElement>(null)
  const [replacing, setReplacing] = useState(false)
  const save = useSaveJevToken()
  const remove = useRemoveJevToken()
  const queryClient = useQueryClient()

  const publish = (saved: AgentSettings) => {
    queryClient.setQueryData(useAgentSettings.getKey(), saved)
    if (field.current) field.current.value = ''
    setReplacing(false)
  }

  const saved = source === JevTokenSource.Settings
  const showField = !saved || replacing
  const busy = save.isPending || remove.isPending

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-col gap-1">
        <span
          id={KEY_TITLE_ID}
          className="font-semibold text-[15.5px] text-cyanotype-ink"
        >
          Ultrafast access key
        </span>
        <span id={KEY_DESCRIPTION_ID} className="text-[14px] text-ink-2">
          {saved
            ? 'A key is saved, so your AI agents can use Ultrafast.'
            : source === JevTokenSource.Environment
              ? 'BrowserOS already has a key from its own setup. A key saved here takes precedence over it.'
              : 'Only needed if you joined the Ultrafast early access. Without a key, your AI agents don’t get Ultrafast.'}
        </span>
      </div>

      {showField ? (
        <form
          className="flex flex-wrap items-start gap-2"
          onSubmit={(event) => {
            event.preventDefault()
            const token = field.current?.value.trim()
            if (!token) return
            save.mutate({ token }, { onSuccess: publish })
          }}
        >
          <Input
            ref={field}
            id={KEY_INPUT_ID}
            type="password"
            autoComplete="off"
            spellCheck={false}
            aria-labelledby={KEY_TITLE_ID}
            aria-describedby={KEY_DESCRIPTION_ID}
            placeholder="Paste your key"
            disabled={busy}
            className="h-10 max-w-sm flex-1"
          />
          <Button type="submit" disabled={busy} className="font-semibold">
            Save
          </Button>
          {saved ? (
            <Button
              type="button"
              variant="ghost"
              disabled={busy}
              onClick={() => {
                if (field.current) field.current.value = ''
                setReplacing(false)
              }}
            >
              Cancel
            </Button>
          ) : null}
        </form>
      ) : (
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="secondary"
            size="sm"
            disabled={busy}
            onClick={() => setReplacing(true)}
            className="font-semibold"
          >
            Replace
          </Button>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={busy}
            onClick={() => remove.mutate(undefined, { onSuccess: publish })}
            className="font-semibold"
          >
            Remove
          </Button>
        </div>
      )}

      <p className="text-[13px] text-ink-3">
        Your AI agents pick it up the next time they start.
      </p>

      {save.isError || remove.isError ? (
        <p role="alert" className="text-[13.5px] text-red-ink">
          Couldn’t save that. Try again.
        </p>
      ) : null}
    </div>
  )
}
