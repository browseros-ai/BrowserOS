import { ArrowRight, Link2, RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { buildCanonicalMcpCliCommand } from '@/modules/api/mcp-endpoint'
import { ConnectedSummaryCard } from '../components/ConnectedSummaryCard'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { OnboardingCopyBlock } from '../components/OnboardingCopyBlock'
import { StepWrap } from '../components/StepWrap'
import type { ConnectPhase } from '../onboarding-v2.types'

interface ConnectStepProps {
  phase: ConnectPhase
  onAddToClaude: () => void
  onContinue: () => void
}

/**
 * Step 2. Three sub-phases driven by `phase`:
 *
 *   idle       : "Add to Claude" button + canonical CLI snippet fallback
 *   connecting : disabled button with a spinner
 *   connected  : green success card + "You're set" CTA
 *
 * The CLI snippet is the exact string the MCP page advertises so a
 * user who reaches onboarding via the reconnect path copies the same
 * command they would copy from /mcp.
 */
export function ConnectStep({
  phase,
  onAddToClaude,
  onContinue,
}: ConnectStepProps) {
  const cli = buildCanonicalMcpCliCommand()
  const isConnecting = phase === 'connecting'
  const isConnected = phase === 'connected'

  return (
    <StepWrap>
      <DisplayHeading>
        Connect to <Em>Claude</Em>.
      </DisplayHeading>
      <StepCopy>
        BrowserOS shows up inside Claude as a connector. One click. No extension
        handshake to fail.
      </StepCopy>

      {!isConnected && (
        <>
          <Button
            type="button"
            size="lg"
            onClick={onAddToClaude}
            disabled={isConnecting}
          >
            {isConnecting ? (
              <>
                <RefreshCw className="size-4 animate-spin" />
                Connecting...
              </>
            ) : (
              <>
                <Link2 className="size-4" />
                Add to Claude
              </>
            )}
          </Button>
          <div className="my-5 flex items-center gap-3 text-[12px] text-ink-4">
            <div className="h-px flex-1 bg-border-2" />
            or use the CLI
            <div className="h-px flex-1 bg-border-2" />
          </div>
          <OnboardingCopyBlock text={cli} />
        </>
      )}

      {isConnected && (
        <>
          <ConnectedSummaryCard />
          <Button type="button" size="lg" onClick={onContinue}>
            <ArrowRight className="size-4" />
            You&rsquo;re set
          </Button>
        </>
      )}
    </StepWrap>
  )
}
