import { PlugZap } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { StarterPromptTile } from '../components/StarterPromptTile'
import { StepWrap } from '../components/StepWrap'
import { STARTER_PROMPTS } from '../onboarding-v2.helpers'

interface ReadyStepProps {
  onDone: () => void
}

const CONNECT_STEPS = [
  <>
    Open the MCP page and click{' '}
    <strong className="font-semibold">Connect</strong> next to your tool.
  </>,
  <>Restart Claude Code once, so it picks up the connection.</>,
  <>Paste a task into Claude Code&rsquo;s chat:</>,
]

/**
 * Final onboarding step. The connection itself happens on the MCP page, so this
 * step's job is to say exactly what to do there and where the copied prompt
 * goes — naming Claude Code rather than leaving the destination implied. The
 * restart gets its own step because an agent that has not reloaded its MCP
 * config looks broken rather than unconnected.
 */
export function ReadyStep({ onDone }: ReadyStepProps) {
  return (
    <StepWrap>
      <DisplayHeading>
        Last step: <Em>connect.</Em>
      </DisplayHeading>
      <StepCopy>
        Your agents can&rsquo;t reach this browser yet. Three things:
      </StepCopy>
      <ol className="mb-5 flex flex-col gap-2.5">
        {CONNECT_STEPS.map((content, index) => (
          <li
            // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length static copy, no DOM identity to preserve
            key={index}
            className="flex items-start gap-2.5 text-[13.5px] text-ink-2"
          >
            <span className="mt-px flex size-[19px] shrink-0 items-center justify-center rounded-md bg-accent font-bold text-[11px] text-card">
              {index + 1}
            </span>
            <span>{content}</span>
          </li>
        ))}
      </ol>
      <div className="mb-6 flex flex-col gap-2.5">
        {STARTER_PROMPTS.slice(0, 2).map((prompt) => (
          <StarterPromptTile key={prompt} prompt={prompt} />
        ))}
      </div>
      <Button type="button" size="lg" onClick={onDone}>
        <PlugZap className="size-4" />
        Open the MCP page
      </Button>
    </StepWrap>
  )
}
