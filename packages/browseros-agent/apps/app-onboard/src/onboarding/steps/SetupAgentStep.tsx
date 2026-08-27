import {
  Anthropic,
  Claude,
  Cohere,
  DeepSeek,
  Gemini,
  Github,
  Grok,
  Groq,
  Kimi,
  Mistral,
  Ollama,
  OpenAI,
  OpenRouter,
  Perplexity,
  Qwen,
} from '@lobehub/icons'
import { ArrowRight } from 'lucide-react'
import type { ComponentType } from 'react'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { Hermes, OpenClaw } from '../components/AgentBrandMarks'
import { DisplayHeading, Em, StepCopy } from '../components/DisplayHeading'
import { StepWrap } from '../components/StepWrap'

interface SetupAgentStepProps {
  onSetup: () => void
  onLater: () => void
}

type MonoIcon = ComponentType<{ size?: number }>
interface IconEntry {
  label: string
  Icon: MonoIcon
}

// Real brand marks from @lobehub/icons (the same source the app uses). The
// default export is the mono icon: a single-path currentColor SVG, so it carries
// no data: URLs or remote refs and stays inside the WebUI resource contract.
const LLM_PROVIDERS: IconEntry[] = [
  { label: 'OpenAI', Icon: OpenAI },
  { label: 'Anthropic', Icon: Anthropic },
  { label: 'Google Gemini', Icon: Gemini },
  { label: 'Mistral', Icon: Mistral },
  { label: 'DeepSeek', Icon: DeepSeek },
  { label: 'xAI Grok', Icon: Grok },
  { label: 'Ollama', Icon: Ollama },
  { label: 'OpenRouter', Icon: OpenRouter },
  { label: 'Moonshot Kimi', Icon: Kimi },
  { label: 'Qwen', Icon: Qwen },
  { label: 'Cohere', Icon: Cohere },
  { label: 'Perplexity', Icon: Perplexity },
  { label: 'Groq', Icon: Groq },
]

const CODING_AGENTS: IconEntry[] = [
  { label: 'Claude Code', Icon: Claude },
  { label: 'Codex', Icon: OpenAI },
  { label: 'GitHub Copilot', Icon: Github },
  { label: 'Gemini CLI', Icon: Gemini },
  { label: 'Qwen Code', Icon: Qwen },
  { label: 'OpenClaw', Icon: OpenClaw },
  { label: 'Hermes', Icon: Hermes },
]

function IconChip({ label, Icon }: IconEntry) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <span className="grid size-[42px] place-items-center rounded-[10px] border border-border bg-card text-foreground transition-all duration-150 hover:-translate-y-0.5 hover:border-accent hover:shadow-[0_6px_14px_var(--color-accent-tint)]" />
        }
      >
        <Icon size={23} />
        <span className="sr-only">{label}</span>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}

/**
 * Final step. The actual provider / agent setup lives in the full BrowserOS app
 * at #/settings/ai; this screen shows the breadth of what is supported and hands
 * off. `bridge.complete()` lets the native first-run finish and open the app on
 * that screen. The two exits differ only in intent today; the landing route is
 * decided natively (see the onboarding plan).
 */
export function SetupAgentStep({ onSetup, onLater }: SetupAgentStepProps) {
  return (
    <TooltipProvider>
      <StepWrap>
        <DisplayHeading>
          Set up your <Em>agent</Em>
        </DisplayHeading>
        <StepCopy>
          Connect an LLM provider or a coding agent harness you already use. We
          will open BrowserOS so you can finish.
        </StepCopy>
        <div className="mb-6 flex max-w-[480px] flex-col gap-[18px]">
          <div>
            <div className="mb-2.5 font-bold text-[11px] text-ink-3 uppercase tracking-[0.08em]">
              Any LLM provider
            </div>
            <div className="flex flex-wrap gap-2">
              {LLM_PROVIDERS.map((entry) => (
                <IconChip key={entry.label} {...entry} />
              ))}
              <span className="grid size-[42px] place-items-center rounded-[10px] bg-accent-tint font-bold text-[12px] text-accent">
                +40
              </span>
            </div>
          </div>
          <div>
            <div className="mb-2.5 font-bold text-[11px] text-ink-3 uppercase tracking-[0.08em]">
              Or a coding agent harness you already use
            </div>
            <div className="flex flex-wrap gap-2">
              {CODING_AGENTS.map((entry) => (
                <IconChip key={entry.label} {...entry} />
              ))}
            </div>
            <div className="mt-2.5 text-[13px] text-ink-3">
              or any other ACP compatible agent
            </div>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <Button type="button" size="lg" onClick={onSetup}>
            Set up my agent
            <ArrowRight className="size-4" />
          </Button>
          <Button type="button" size="lg" variant="ghost" onClick={onLater}>
            I'll do this later
          </Button>
        </div>
      </StepWrap>
    </TooltipProvider>
  )
}
