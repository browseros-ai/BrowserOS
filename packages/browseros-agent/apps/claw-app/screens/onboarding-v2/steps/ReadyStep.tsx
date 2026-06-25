import { Sparkles } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { StarterPromptTile } from '../components/StarterPromptTile'
import { StepWrap } from '../components/StepWrap'
import { STARTER_PROMPTS } from '../onboarding-v2.helpers'

interface ReadyStepProps {
  onDone: () => void
}

/**
 * Step 3. Two starter prompt tiles and an "Open BrowserOS" CTA that
 * navigates the user home. `onDone` lives on the parent so the
 * wiring person can swap navigation behaviour without touching the
 * step component.
 */
export function ReadyStep({ onDone }: ReadyStepProps) {
  return (
    <StepWrap>
      <DisplayHeading>
        You&rsquo;re <Em>set</Em>.
      </DisplayHeading>
      <StepCopy>
        Open Claude and try one of these. The task runs here, in BrowserOS. You
        watch, approve, and audit.
      </StepCopy>
      <div className="mb-6 flex flex-col gap-2.5">
        {STARTER_PROMPTS.slice(0, 2).map((prompt) => (
          <StarterPromptTile key={prompt} prompt={prompt} />
        ))}
      </div>
      <Button type="button" size="lg" onClick={onDone}>
        <Sparkles className="size-4" />
        Open BrowserOS
      </Button>
    </StepWrap>
  )
}
