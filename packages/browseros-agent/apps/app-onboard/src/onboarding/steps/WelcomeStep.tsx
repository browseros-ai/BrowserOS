import { ArrowRight } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { StepWrap } from '../components/StepWrap'

interface WelcomeStepProps {
  onPrimary: () => void
  onSkip: () => void
}

/** Opening step: names the two things onboarding will do, then gets out of the way. */
export function WelcomeStep({ onPrimary, onSkip }: WelcomeStepProps) {
  return (
    <StepWrap>
      <DisplayHeading>
        Welcome to <Em>BrowserOS</Em>
      </DisplayHeading>
      <StepCopy>
        Two quick steps to get set up: bring your browser over, then connect
        your agent.
      </StepCopy>
      <div className="flex flex-wrap items-center gap-3">
        <Button type="button" size="lg" onClick={onPrimary}>
          Get started
          <ArrowRight className="size-4" />
        </Button>
        <Button type="button" size="lg" variant="ghost" onClick={onSkip}>
          Skip for now
        </Button>
      </div>
    </StepWrap>
  )
}
