import { Zap } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { StepWrap } from '../components/StepWrap'

interface WelcomeStepProps {
  onPrimary: () => void
  onSkip: () => void
}

/**
 * Step 0. Welcome copy + two CTAs. Primary advances into the full
 * flow, the quieter CTA skips ahead to the Ready step for users who
 * have done this before and just need the canonical CLI snippet
 * (Phase 3 wires this to step 3 because Connect carries the CLI; the
 * wiring person can re-route to a reconnect-specific screen later).
 */
export function WelcomeStep({ onPrimary, onSkip }: WelcomeStepProps) {
  return (
    <StepWrap>
      <DisplayHeading>
        The browser your agents <Em>drive</Em>.
      </DisplayHeading>
      <StepCopy>
        Logged in as you, fast, and under your control. Set-up takes about two
        minutes. Import your logins, connect to Claude, and run your first task.
      </StepCopy>
      <div className="flex flex-wrap items-center gap-3">
        <Button type="button" size="lg" onClick={onPrimary}>
          <Zap className="size-4" />
          Set up . about 2 min
        </Button>
        <Button type="button" size="lg" variant="ghost" onClick={onSkip}>
          I&rsquo;ve done this before . reconnect
        </Button>
      </div>
    </StepWrap>
  )
}
