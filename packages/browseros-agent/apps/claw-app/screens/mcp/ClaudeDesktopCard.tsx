import { ArrowUpRight, CircleCheck, LoaderCircle } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { HarnessIcon } from '@/components/harness/HarnessIcon'
import { Button, ButtonLink } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { GET_CLAUDE_URL } from '@/screens/cockpit/cockpit-onboarding.helpers'
import { AgentRow } from './ConnectionRow'
import type { ClaudeSetupState } from './claude-desktop.helpers'
import { useAddToClaude } from './claude-desktop.hooks'
import { InstallExtensionDialog } from './InstallExtensionDialog'

const SUB: Record<ClaudeSetupState, string> = {
  checking: 'Needs a one-time extension.',
  'no-claude': 'Install the Claude desktop app first, then come back here.',
  add: 'Adds neo to Claude. You confirm with one click in Claude.',
  update: 'A newer version of neo for Claude is ready.',
  waiting: 'Claude is open. Click Install there to finish.',
  stuck: 'Didn’t see Claude ask? Try again, or set it up step by step.',
  connected: 'Ready to use',
}

/**
 * The Claude desktop app takes neo as an extension. Adding it hands the
 * extension to Claude, which asks the person to confirm; the step-by-step
 * guide stays one click away for when that does not work.
 */
export function ClaudeDesktopCard() {
  const [guideOpen, setGuideOpen] = useState(false)
  const claude = useAddToClaude()
  const guideLink = (
    <Button
      variant="ghost"
      size="sm"
      onClick={() => setGuideOpen(true)}
      className="font-semibold text-ink-2"
    >
      Step by step
    </Button>
  )

  return (
    <>
      <AgentRow
        className="bg-bg-sunken"
        logo={<HarnessIcon harness="Claude Code" className="size-[22px]" />}
        name="Claude desktop app"
        sub={SUB[claude.state]}
        action={
          <ClaudeAction
            state={claude.state}
            isOpening={claude.isOpening}
            onAdd={claude.add}
            guideLink={guideLink}
          />
        }
      />
      <InstallExtensionDialog open={guideOpen} onOpenChange={setGuideOpen} />
    </>
  )
}

// ---------------------------------------------------------------------------

function ClaudeAction({
  state,
  isOpening,
  onAdd,
  guideLink,
}: {
  state: ClaudeSetupState
  isOpening: boolean
  onAdd: () => void
  guideLink: ReactNode
}) {
  if (state === 'checking') return null
  if (state === 'connected') {
    return (
      <span className="inline-flex items-center gap-1.5 font-semibold text-[14px] text-green-ink">
        <CircleCheck aria-hidden className="size-4" />
        Connected
      </span>
    )
  }
  if (state === 'no-claude') {
    return (
      <ButtonLink
        href={GET_CLAUDE_URL}
        target="_blank"
        rel="noopener noreferrer"
        size="sm"
        className="font-semibold"
      >
        Get Claude
        <ArrowUpRight aria-hidden className="size-[15px]" />
      </ButtonLink>
    )
  }
  if (state === 'waiting') {
    return (
      <span className="flex items-center gap-2">
        <span
          role="status"
          className="inline-flex items-center gap-1.5 font-semibold text-[14px] text-ink-2"
        >
          <LoaderCircle aria-hidden className="size-4 animate-spin" />
          Waiting for Claude
        </span>
        {guideLink}
      </span>
    )
  }
  return (
    <span className="flex items-center gap-2">
      {state === 'stuck' ? guideLink : null}
      <StatefulButton
        size="sm"
        state={isOpening ? 'loading' : 'idle'}
        loadingText="Opening Claude"
        disabled={isOpening}
        onClick={onAdd}
        className="font-semibold"
      >
        {state === 'update'
          ? 'Update'
          : state === 'stuck'
            ? 'Try again'
            : 'Add to Claude'}
      </StatefulButton>
    </span>
  )
}
