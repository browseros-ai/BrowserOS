import { Loader2, Pencil } from 'lucide-react'
import { type FC, useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { getProxyPort } from '@/lib/browseros/helpers'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'
import { MCP_PROXY_PORT_CHANGED_EVENT } from '@/lib/constants/analyticsEvents'
import { track } from '@/lib/metrics/track'
import { waitForServerHealth } from './server-health'
import { PROXY_PORT_MIN, parseProxyPort } from './server-port-editor.helpers'

export interface ServerPortEditorProps {
  onPortChanged?: () => void
}

async function readCurrentPort(): Promise<number> {
  try {
    return await getProxyPort()
  } catch {
    // Pref unset or BrowserOS API unavailable — fall back to the default port
    return PROXY_PORT_MIN
  }
}

/** Popover for editing the MCP proxy port and waiting for BrowserOS to rebind it. */
export const ServerPortEditor: FC<ServerPortEditorProps> = ({
  onPortChanged,
}) => {
  const [open, setOpen] = useState(false)
  const [value, setValue] = useState('')
  const [currentPort, setCurrentPort] = useState<number | null>(null)
  const [isSaving, setIsSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const isMountedRef = useRef(true)

  useEffect(() => {
    isMountedRef.current = true
    return () => {
      isMountedRef.current = false
    }
  }, [])

  const handleOpenChange = async (next: boolean) => {
    if (!next && isSaving) return
    setOpen(next)
    if (next) {
      setError(null)
      const port = await readCurrentPort()
      if (!isMountedRef.current) return
      setCurrentPort(port)
      setValue(String(port))
    }
  }

  const parsed = parseProxyPort(value)
  const canSave = parsed.ok && !isSaving && parsed.port !== currentPort

  const handleSave = async () => {
    const result = parseProxyPort(value)
    if (!result.ok) {
      setError(result.error)
      return
    }
    setIsSaving(true)
    setError(null)
    try {
      const success = await getBrowserOSAdapter().setPref(
        BROWSEROS_PREFS.PROXY_PORT,
        result.port,
      )
      if (!success) {
        throw new Error('Failed to update port')
      }
      const healthy = await waitForServerHealth()
      // Bail if the settings page unmounted mid-poll — don't poke a stale tree.
      if (!isMountedRef.current) return
      if (healthy) {
        track(MCP_PROXY_PORT_CHANGED_EVENT, { port: result.port })
        setCurrentPort(result.port)
        toast.success('Server port updated', {
          description: `MCP clients now connect on port ${result.port}.`,
        })
        onPortChanged?.()
        setOpen(false)
      } else {
        setError(
          "Port saved, but the server didn't restart in time. Quit and relaunch the browser to apply it.",
        )
      }
    } catch (err) {
      if (!isMountedRef.current) return
      setError(err instanceof Error ? err.message : 'Failed to update port')
    } finally {
      if (isMountedRef.current) setIsSaving(false)
    }
  }

  return (
    <Popover open={open} onOpenChange={handleOpenChange}>
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="icon"
          className="shrink-0"
          title="Edit server port"
        >
          <Pencil className="h-4 w-4" />
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80">
        <div className="space-y-3">
          <div className="space-y-1">
            <h4 className="font-medium text-sm">Server port</h4>
            <p className="text-muted-foreground text-xs leading-relaxed">
              The port external MCP clients (Claude Code, Gemini CLI, and
              others) use to connect. Saving restarts the server, so any active
              connections will briefly drop.
            </p>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="proxy-port" className="sr-only">
              Server port
            </Label>
            <Input
              id="proxy-port"
              inputMode="numeric"
              autoComplete="off"
              autoFocus
              value={value}
              aria-invalid={!parsed.ok && value.trim() !== ''}
              aria-describedby={error ? 'proxy-port-error' : undefined}
              disabled={isSaving}
              onChange={(e) => {
                setValue(e.target.value)
                setError(null)
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && canSave) handleSave()
              }}
            />
            {error && (
              <p id="proxy-port-error" className="text-destructive text-xs">
                {error}
              </p>
            )}
          </div>
          <div className="flex justify-end gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setOpen(false)}
              disabled={isSaving}
            >
              Cancel
            </Button>
            <Button size="sm" onClick={handleSave} disabled={!canSave}>
              {isSaving ? (
                <>
                  <Loader2 className="mr-2 h-4 w-4 animate-spin" />
                  Restarting…
                </>
              ) : (
                'Save'
              )}
            </Button>
          </div>
        </div>
      </PopoverContent>
    </Popover>
  )
}
