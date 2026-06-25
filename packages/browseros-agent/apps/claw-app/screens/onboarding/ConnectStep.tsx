import {
  Check,
  CheckCircle2,
  ChevronRight,
  Copy,
  Link2,
  Loader2,
} from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  type ConnectResult,
  useConnectToClaude,
} from '@/modules/api/onboarding.hooks'
import { CLAUDE_MCP_CLI } from './onboarding.helpers'

interface ConnectStepProps {
  onContinue: (result: ConnectResult) => void
}

/**
 * Step 2. One-click "Add to Claude" with a CLI fallback for users
 * whose harness doesn't speak the connector-install API yet.
 */
export function ConnectStep({ onContinue }: ConnectStepProps) {
  const connect = useConnectToClaude()
  const [result, setResult] = useState<ConnectResult | null>(null)

  const startConnect = () => {
    connect.mutate(undefined, {
      onSuccess: (data) => setResult(data),
    })
  }

  return (
    <div className="flex w-full max-w-[560px] flex-col">
      <h1 className="mb-3 font-extrabold text-[34px] text-ink leading-[1.05] tracking-tight">
        Connect to{' '}
        <em className="font-['Newsreader',serif] font-medium text-accent italic">
          Claude
        </em>
        .
      </h1>
      <p className="mb-5 max-w-[470px] text-ink-2 text-sm leading-relaxed">
        BrowserOS shows up inside Claude as a connector. One click, no extension
        handshake to fail.
      </p>

      {!result && (
        <>
          <Button
            type="button"
            size="lg"
            onClick={startConnect}
            disabled={connect.isPending}
            className="self-start"
          >
            {connect.isPending ? (
              <>
                <Loader2 className="size-4 animate-spin" />
                Connecting…
              </>
            ) : (
              <>
                <Link2 className="size-4" />
                Add to Claude
              </>
            )}
          </Button>

          <div className="my-5 flex items-center gap-3 text-ink-4 text-xs">
            <div className="h-px flex-1 bg-border-2" />
            or use the CLI
            <div className="h-px flex-1 bg-border-2" />
          </div>

          <CopyableCli command={CLAUDE_MCP_CLI} />
        </>
      )}

      {result && (
        <div className="flex flex-col gap-4">
          <div className="flex items-center gap-3 rounded-2xl border border-[#BFE3CC] bg-green-tint p-5">
            <span className="flex size-8 items-center justify-center rounded-lg bg-card text-green">
              <CheckCircle2 className="size-5" />
            </span>
            <div>
              <div className="font-bold text-ink text-sm">
                Connected to Claude
              </div>
              <div className="text-ink-2 text-xs">
                {result.toolCount} browser tools available · scope:{' '}
                {result.scope}
              </div>
            </div>
          </div>
          <Button
            type="button"
            size="lg"
            onClick={() => onContinue(result)}
            className="self-start"
          >
            <ChevronRight className="size-4" />
            You're set
          </Button>
        </div>
      )}
    </div>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components, private to this step.
 * -------------------------------------------------------------------------*/

function CopyableCli({ command }: { command: string }) {
  const [copied, setCopied] = useState(false)
  const onCopy = async () => {
    try {
      await navigator.clipboard.writeText(command)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }
  return (
    <div className="flex items-center gap-2.5 rounded-xl bg-[#15140F] px-3 py-3">
      <span className="font-mono text-[#6FCF8E] text-xs">$</span>
      <code className="min-w-0 flex-1 truncate font-mono text-[#EDEAE2] text-xs">
        {command}
      </code>
      <button
        type="button"
        onClick={onCopy}
        aria-label="Copy CLI command"
        className={cn(
          'flex items-center gap-1.5 rounded-md bg-white/10 px-2 py-1 font-semibold text-[11px] text-white hover:bg-white/20',
        )}
      >
        {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
        {copied ? 'Copied' : 'Copy'}
      </button>
    </div>
  )
}
