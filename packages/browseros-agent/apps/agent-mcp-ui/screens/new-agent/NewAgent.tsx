import { zodResolver } from '@hookform/resolvers/zod'
import { ArrowLeft } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { useForm } from 'react-hook-form'
import { Form } from '@/components/ui/form'
import type { AgentRow } from '@/modules/api/agents.hooks'
import { AclRulesSection } from './AclRulesSection'
import { ApprovalsSection } from './ApprovalsSection'
import { ConnectorPreviewRail } from './ConnectorPreviewRail'
import { CopyFromExistingCard } from './CopyFromExistingCard'
import { HarnessSection } from './HarnessSection'
import { LoginsSection } from './LoginsSection'
import { useNewAgentData } from './new-agent.data'
import { SEED_ACL_RULES } from './new-agent.helpers'
import {
  type NewAgentValues,
  newAgentDefaults,
  newAgentSchema,
} from './new-agent.schemas'

export function NewAgent() {
  const { agents, createAgent, navigate } = useNewAgentData()

  const form = useForm<NewAgentValues>({
    resolver: zodResolver(newAgentSchema),
    defaultValues: {
      ...newAgentDefaults,
      aclRuleIds: SEED_ACL_RULES.map((rule) => rule.id),
    },
    mode: 'onSubmit',
  })

  const [cloneFromId, setCloneFromId] = useState<string | null>(null)

  const handleClone = (agent: AgentRow) => {
    setCloneFromId(agent.id)
    form.setValue('harness', agent.harness, { shouldDirty: true })
    if (form.getValues('name').trim() === '') {
      form.setValue('name', `Copy of ${agent.label}`, { shouldDirty: true })
    }
  }

  const onSubmit = (values: NewAgentValues) => createAgent.mutate(values)

  return (
    <div className="flex h-full min-h-0 flex-1 bg-bg-canvas">
      <Form {...form}>
        <form
          onSubmit={form.handleSubmit(onSubmit)}
          className="flex h-full min-h-0 flex-1"
        >
          <div className="flex h-full min-h-0 flex-1 flex-col overflow-y-auto">
            <header className="sticky top-0 z-10 flex items-center gap-3 border-border border-b bg-card px-6 py-3.5">
              <button
                type="button"
                onClick={() => navigate('/agents')}
                className="flex items-center gap-1.5 font-semibold text-ink-2 text-sm hover:text-ink"
              >
                <ArrowLeft className="size-4" />
                Agents
              </button>
              <span className="h-5 w-px bg-border-2" />
              <span className="font-extrabold text-base text-ink tracking-tight">
                Add an agent
              </span>
              <span className="text-ink-3 text-xs">
                — connect a harness to BrowserOS
              </span>
            </header>

            <div className="mx-auto flex w-full max-w-[600px] flex-col gap-6 px-6 py-6 pb-20">
              <CopyFromExistingCard
                agents={agents}
                selectedId={cloneFromId}
                onClone={handleClone}
              />
              <NumberedSection
                n={1}
                title="Harness"
                sub="Name the connector and pick the agent that will drive the browser."
              >
                <HarnessSection />
              </NumberedSection>
              <NumberedSection
                n={2}
                title="Logins (profile)"
                sub="Which of your saved sessions this agent may use. Passwords never leave this Mac."
              >
                <LoginsSection />
              </NumberedSection>
              <NumberedSection
                n={3}
                title="Tool approvals"
                sub="What this agent does automatically vs. what needs your OK."
              >
                <ApprovalsSection />
              </NumberedSection>
              <NumberedSection
                n={4}
                title="ACL rules"
                sub="Site-level blocks enforced at the browser, even under prompt injection."
              >
                <AclRulesSection />
              </NumberedSection>
            </div>
          </div>

          <ConnectorPreviewRail
            createdAgent={createAgent.data}
            isCreating={createAgent.isPending}
            onDone={() => navigate('/agents')}
          />
        </form>
      </Form>
    </div>
  )
}

/* ---------------------------------------------------------------------------
 * Sub-components — kept private to this module to stay under the line budget.
 * -------------------------------------------------------------------------*/

interface NumberedSectionProps {
  n: number
  title: string
  sub: string
  children: ReactNode
}

function NumberedSection({ n, title, sub, children }: NumberedSectionProps) {
  return (
    <section>
      <header className="mb-3 flex items-center gap-2.5">
        <span className="flex size-[22px] shrink-0 items-center justify-center rounded-full bg-ink font-bold text-[11.5px] text-card">
          {n}
        </span>
        <div>
          <div className="font-bold text-ink text-sm">{title}</div>
          <div className="text-ink-3 text-xs">{sub}</div>
        </div>
      </header>
      <div className="pl-8">{children}</div>
    </section>
  )
}
