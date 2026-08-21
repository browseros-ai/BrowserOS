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
 * Final onboarding step. Connecting is no longer homework: the browser wires
 * itself up to the agents already installed on this machine, so the screen
 * states that and hands over a prompt to paste — the copy buttons are the
 * action, not a numbered checklist.
 *
 * The restart survives as a footnote rather than a step because an agent that
 * has not reloaded its MCP config looks broken rather than unconnected, and
 * that is the one way a finished setup still reads as failed. Leading with it
 * would undo the point of the screen.
 */
export function ReadyStep({ onDone }: ReadyStepProps) {
  return (
    <StepWrap>
      <DisplayHeading>
        All <Em>set!</Em>
      </DisplayHeading>
      <StepCopy>
        We connected this browser to the agents already installed on your
        machine. Try it out &mdash; copy one of these into your agent:
      </StepCopy>
      <div className="mb-4 flex flex-col gap-2.5">
        {STARTER_PROMPTS.slice(0, 2).map((prompt) => (
          <StarterPromptTile key={prompt} prompt={prompt} />
        ))}
      </div>
      <p className="mb-6 max-w-[470px] text-[12.5px] text-ink-3 leading-[1.5]">
        Agent doesn&rsquo;t see this browser yet? Restart it once, so it picks
        up the connection.
      </p>
      <Button type="button" size="lg" onClick={onDone}>
        <PlugZap className="size-4" />
        Open the MCP page
      </Button>
    </StepWrap>
  )
}
