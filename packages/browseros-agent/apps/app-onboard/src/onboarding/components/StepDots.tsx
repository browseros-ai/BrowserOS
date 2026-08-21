import { cn } from '@/lib/utils'

interface StepDotsProps {
  step: number
  total: number
}

/** Renders compact step progress dots for the onboarding frame. */
export function StepDots({ step, total }: StepDotsProps) {
  return (
    <div
      className="flex items-center gap-[7px]"
      role="progressbar"
      aria-label="Onboarding progress"
      aria-valuemin={1}
      aria-valuemax={total}
      aria-valuenow={step + 1}
    >
      {Array.from({ length: total }).map((_, idx) => {
        const isActive = idx === step
        const isDone = idx < step
        return (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: dots are pure decoration with a fixed total; no DOM identity to preserve
            key={idx}
            data-step-dot="true"
            aria-hidden
            className={cn(
              'h-[7px] rounded-full transition-all duration-300',
              isActive ? 'w-[22px] bg-accent' : 'w-[7px]',
              !isActive && isDone && 'bg-accent-tint-2',
              !isActive && !isDone && 'bg-border-2',
            )}
          />
        )
      })}
    </div>
  )
}
