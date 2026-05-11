import { useQueryClient } from '@tanstack/react-query'
import {
  Briefcase,
  Check,
  Loader2,
  MessageSquare,
  RotateCcw,
  Scale,
  Send,
  SmilePlus,
  Zap,
} from 'lucide-react'
import type { FC } from 'react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Textarea } from '@/components/ui/textarea'
import { openSidePanelWithSearch } from '@/lib/messaging/sidepanel/openSidepanelWithSearch'
import { type SoulPresetId, soulPresets } from '@/lib/onboarding/soulPresets'
import { useRpcClient } from '@/lib/rpc/RpcClientProvider'
import { sentry } from '@/lib/sentry/sentry'
import { cn } from '@/lib/utils'
import { SOUL_QUERY_KEY } from './useSoulContent'

interface Example {
  label: string
  query: string
}

const SOUL_EXAMPLES: Example[] = [
  {
    label: 'Set your tone',
    query:
      'Be more casual and direct with me. Skip formalities and just get to the point.',
  },
  {
    label: 'Add a boundary',
    query:
      'Never auto-close my tabs without asking first. Add this to your soul.',
  },
  {
    label: 'Change personality',
    query:
      'I want you to be witty and slightly sarcastic, like a smart coworker who enjoys their job.',
  },
]

const presetIcons: Record<SoulPresetId, typeof Scale> = {
  balanced: Scale,
  professional: Briefcase,
  friendly: SmilePlus,
  minimal: Zap,
}

export const SoulExamples: FC = () => {
  const [editingQuery, setEditingQuery] = useState('')
  const [dialogOpen, setDialogOpen] = useState(false)
  const [resetDialogOpen, setResetDialogOpen] = useState(false)
  const [selectedPreset, setSelectedPreset] = useState<SoulPresetId>('balanced')
  const [isResetting, setIsResetting] = useState(false)
  const rpcClient = useRpcClient()
  const queryClient = useQueryClient()

  const handleTryIt = (query: string) => {
    setEditingQuery(query)
    setDialogOpen(true)
  }

  const handleSend = () => {
    if (!editingQuery.trim()) return
    openSidePanelWithSearch('open', {
      query: editingQuery.trim(),
      mode: 'agent',
    })
    setDialogOpen(false)
  }

  const handleReset = async () => {
    const preset = soulPresets.find((p) => p.id === selectedPreset)
    if (!preset) return

    setIsResetting(true)
    try {
      await rpcClient.soul.$put({
        json: { content: preset.content },
      })
      await queryClient.invalidateQueries({ queryKey: [SOUL_QUERY_KEY] })
      setResetDialogOpen(false)
    } catch (e) {
      sentry.captureException(e, {
        extra: { message: 'Failed to reset soul' },
      })
    } finally {
      setIsResetting(false)
    }
  }

  return (
    <div className="space-y-3">
      <div>
        <h3 className="font-medium text-sm">Shape your Sup's soul</h3>
        <p className="mt-1 text-muted-foreground text-xs">
          Try these prompts to customize how your Sup behaves. Edit the
          message before sending.
        </p>
      </div>

      <div className="grid gap-2">
        {SOUL_EXAMPLES.map((example) => (
          <div
            key={example.label}
            className="flex items-center justify-between rounded-lg border border-border bg-card p-3 transition-colors hover:bg-muted/50"
          >
            <div className="mr-3 min-w-0 flex-1">
              <p className="font-medium text-sm">{example.label}</p>
              <p className="mt-0.5 truncate text-muted-foreground text-xs">
                {example.query}
              </p>
            </div>
            <Button
              variant="outline"
              size="sm"
              className="shrink-0 gap-1.5"
              onClick={() => handleTryIt(example.query)}
            >
              <MessageSquare className="h-3.5 w-3.5" />
              Try it
            </Button>
          </div>
        ))}

        <div className="flex items-center justify-between rounded-lg border border-border bg-card p-3 transition-colors hover:bg-muted/50">
          <div className="mr-3 min-w-0 flex-1">
            <p className="font-medium text-sm">Reset your soul</p>
            <p className="mt-0.5 truncate text-muted-foreground text-xs">
              Start fresh with one of the preset personalities
            </p>
          </div>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0 gap-1.5"
            onClick={() => setResetDialogOpen(true)}
          >
            <RotateCcw className="h-3.5 w-3.5" />
            Reset
          </Button>
        </div>
      </div>

      <Dialog open={dialogOpen} onOpenChange={setDialogOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Edit message</DialogTitle>
            <DialogDescription>
              Customize the prompt before sending it to your Sup.
            </DialogDescription>
          </DialogHeader>
          <Textarea
            value={editingQuery}
            onChange={(e) => setEditingQuery(e.target.value)}
            rows={4}
            className="resize-none"
          />
          <DialogFooter>
            <Button variant="outline" onClick={() => setDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleSend}
              disabled={!editingQuery.trim()}
              className="gap-1.5"
            >
              <Send className="h-3.5 w-3.5" />
              Send
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog open={resetDialogOpen} onOpenChange={setResetDialogOpen}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>Reset your soul</DialogTitle>
            <DialogDescription>
              Pick a personality preset. This will replace your current soul
              file.
            </DialogDescription>
          </DialogHeader>
          <div className="grid grid-cols-2 gap-2">
            {soulPresets.map((preset) => {
              const Icon = presetIcons[preset.id]
              const isSelected = selectedPreset === preset.id

              return (
                <button
                  key={preset.id}
                  type="button"
                  onClick={() => setSelectedPreset(preset.id)}
                  className={cn(
                    'relative flex flex-col items-center gap-2 rounded-lg border-2 p-4 text-center transition-all',
                    isSelected
                      ? 'border-[var(--accent-orange)] bg-[var(--accent-orange)]/5'
                      : 'border-border hover:border-[var(--accent-orange)]/50',
                  )}
                >
                  {isSelected && (
                    <div className="absolute top-2 right-2 flex size-4 items-center justify-center rounded-full bg-[var(--accent-orange)]">
                      <Check className="size-2.5 text-white" />
                    </div>
                  )}
                  <Icon
                    className={cn(
                      'size-5',
                      isSelected
                        ? 'text-[var(--accent-orange)]'
                        : 'text-muted-foreground',
                    )}
                  />
                  <div>
                    <div className="font-medium text-sm">{preset.name}</div>
                    <div className="text-muted-foreground text-xs">
                      {preset.description}
                    </div>
                  </div>
                </button>
              )
            })}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setResetDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleReset}
              disabled={isResetting}
              className="gap-1.5 bg-[var(--accent-orange)] text-white hover:bg-[var(--accent-orange)]/90"
            >
              {isResetting ? (
                <Loader2 className="size-3.5 animate-spin" />
              ) : (
                <RotateCcw className="h-3.5 w-3.5" />
              )}
              Reset Soul
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}

