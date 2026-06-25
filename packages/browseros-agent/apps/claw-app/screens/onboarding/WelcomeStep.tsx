import { Bolt, Sparkles } from 'lucide-react'
import { Button } from '@/components/ui/button'

interface WelcomeStepProps {
  onSetup: () => void
  onSkip: () => void
}

/**
 * Step 0. Hero headline + setup CTA + a quiet "I've done this before"
 * skip CTA that drops the user straight into the cockpit.
 */
export function WelcomeStep({ onSetup, onSkip }: WelcomeStepProps) {
  return (
    <div className="flex w-full max-w-[560px] flex-col">
      <h1 className="mb-3 font-extrabold text-[34px] text-ink leading-[1.05] tracking-tight">
        The browser your agents{' '}
        <em className="font-['Newsreader',serif] font-medium text-accent italic">
          drive
        </em>
        .
      </h1>
      <p className="mb-7 max-w-[460px] text-ink-2 text-sm leading-relaxed">
        Logged in as you, fast, and under your control. Set-up takes about two
        minutes: import your logins, connect to Claude, and run your first task.
      </p>
      <div className="flex items-center gap-3">
        <Button type="button" size="lg" onClick={onSetup}>
          <Bolt className="size-4" />
          Set up . about 2 min
        </Button>
        <Button type="button" size="lg" variant="ghost" onClick={onSkip}>
          <Sparkles className="size-4" />
          I've done this before, reconnect
        </Button>
      </div>
    </div>
  )
}
