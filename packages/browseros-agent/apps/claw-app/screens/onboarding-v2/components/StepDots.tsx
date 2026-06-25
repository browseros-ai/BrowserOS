import { cn } from '@/lib/utils'

interface StepDotsProps {
  step: number
  total: number
}

/**
 * Four-dot progress indicator. Active dot is a 22 px elongated pill
 * in accent orange, completed dots are accent-tint-2, future dots are
 * border-2 grey. Width / colour transitions over 300 ms so step
 * changes feel smooth.
 */
export function StepDots({ step, total }: StepDotsProps) {
  return (
    <div className="flex items-center gap-[7px]">
      {Array.from({ length: total }).map((_, idx) => {
        const isActive = idx === step
        const isDone = idx < step
        return (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: dots are pure decoration with a fixed total; no DOM identity to preserve
            key={idx}
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
