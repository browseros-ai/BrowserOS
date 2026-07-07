import { PlugZap } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { StarterPromptTile } from '../components/StarterPromptTile'
import { StepWrap } from '../components/StepWrap'
import { STARTER_PROMPTS } from '../onboarding-v2.helpers'

interface ReadyStepProps {
  onDone: () => void
}

/**
 * Final onboarding step. Confirms the import landed and points at
 * the MCP page for harness link-up. Reached only after a successful
 * import; the reconnect path from Welcome bypasses this step and
 * completes onboarding directly.
 */
export function ReadyStep({ onDone }: ReadyStepProps) {
  return (
    <StepWrap>
      <DisplayHeading>
        Logins <Em>imported.</Em>
      </DisplayHeading>
      <StepCopy>
        One step left. Open the MCP page in BrowserClaw and link your AI: Claude
        Code, Cursor, Codex, or any other. Your agent runs tasks in this
        browser. You watch, approve, and audit.
      </StepCopy>
      <div className="mb-2.5 font-bold text-[12.5px] text-ink-2">
        Once connected, try one of these.
      </div>
      <div className="mb-6 flex flex-col gap-2.5">
        {STARTER_PROMPTS.slice(0, 2).map((prompt) => (
          <StarterPromptTile key={prompt} prompt={prompt} />
        ))}
      </div>
      <Button type="button" size="lg" onClick={onDone}>
        <PlugZap className="size-4" />
        Connect your AI
      </Button>
    </StepWrap>
  )
}
