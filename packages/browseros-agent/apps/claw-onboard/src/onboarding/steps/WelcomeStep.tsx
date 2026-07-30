import { Zap } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { StepWrap } from '../components/StepWrap'

interface WelcomeStepProps {
  onPrimary: () => void
  onSkip: () => void
}

/** Renders the opening onboarding step and setup/reconnect choices. */
export function WelcomeStep({ onPrimary, onSkip }: WelcomeStepProps) {
  return (
    <StepWrap>
      <DisplayHeading>
        Your second browser. For your <Em>agents.</Em>
      </DisplayHeading>
      <StepCopy>
        Not a Chrome replacement. This browser is for your agents, where Claude,
        Codex, and Cursor work.
      </StepCopy>
      <div className="flex flex-wrap items-center gap-3">
        <Button type="button" size="lg" onClick={onPrimary}>
          <Zap className="size-4" />
          Set it up
        </Button>
        <Button type="button" size="lg" variant="ghost" onClick={onSkip}>
          Already set up? Reconnect.
        </Button>
      </div>
    </StepWrap>
  )
}
