import {
  Bot,
  Check,
  CheckCircle2,
  Code,
  Copy,
  Globe,
  Link2,
  Lock,
  MousePointer2,
  Shield,
  Sparkles,
  Terminal,
} from 'lucide-react'
import { type ComponentType, useState } from 'react'
import { useFormContext } from 'react-hook-form'
import { Button } from '@/components/ui/button'
import type { CreatedAgent } from '@/modules/api/agents.hooks'
import {
  buildCliCommand,
  buildMcpUrl,
  countApprovalVerdicts,
  describeLogins,
  toSlug,
} from './new-agent.helpers'
import type { Harness, NewAgentValues } from './new-agent.schemas'

const HARNESS_ICONS: Record<Harness, ComponentType<{ className?: string }>> = {
  'Claude Cowork': Sparkles,
  Codex: Code,
  Hermes: Bot,
  OpenClaw: MousePointer2,
  'Gemini CLI': Terminal,
}

interface ConnectorPreviewRailProps {
  mode: 'create' | 'edit'
  createdAgent: CreatedAgent | undefined
  isMutating: boolean
  /** True once the create or update mutation has resolved. */
  submitted: boolean
  onDone: () => void
}

export function ConnectorPreviewRail({
  mode,
  createdAgent,
  isMutating,
  submitted,
  onDone,
}: ConnectorPreviewRailProps) {
  const form = useFormContext<NewAgentValues>()
  const values = form.watch()
  const [copied, setCopied] = useState(false)

  const slug = toSlug(values.name || values.harness)
  const mcpUrl = createdAgent?.mcpUrl ?? buildMcpUrl(slug)
  const cliCommand = createdAgent?.cliCommand ?? buildCliCommand(slug)
  const logins = describeLogins(values.loginMode, values.selectedSites.length)
  const verdicts = countApprovalVerdicts(values.approvals)
  const HarnessIcon = HARNESS_ICONS[values.harness]
  const aclCount = values.aclRuleIds.length
  const nameInvalid = values.name.trim().length === 0
  const isEdit = mode === 'edit'
  const idleCtaLabel = isEdit
    ? `Save changes to ${values.harness}`
    : `Add to ${values.harness}`
  const pendingCtaLabel = isEdit ? 'Saving…' : 'Adding…'
  const successHeadline = isEdit
    ? `${values.harness} updated`
    : `Added to ${values.harness}`
  const successDetail = isEdit
    ? 'Connector settings synced'
    : 'Endpoint registered · scope: user'

  const copyMcpUrl = async () => {
    try {
      await navigator.clipboard.writeText(mcpUrl)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1500)
    } catch {
      setCopied(false)
    }
  }

  return (
    <aside className="flex w-[320px] shrink-0 flex-col gap-3 overflow-y-auto border-border border-l bg-card p-5">
      <span className="font-bold text-[11px] text-ink-4 uppercase tracking-wider">
        This connector
      </span>

      <div className="rounded-2xl border border-border-2 bg-card p-4 shadow-sm">
        <div className="mb-3 flex items-center gap-2.5">
          <span className="flex size-8 items-center justify-center rounded-lg bg-accent-tint text-accent">
            <HarnessIcon className="size-4" />
          </span>
          <div className="min-w-0 flex-1">
            <div className="truncate font-bold text-ink text-sm">
              {values.name || 'Untitled connector'}
            </div>
            <div className="text-ink-3 text-xs">{values.harness}</div>
          </div>
        </div>
        <ul className="flex flex-col gap-1.5 text-ink-2 text-xs">
          <li className="flex items-center gap-1.5">
            <Lock className="size-3 text-ink-3" />
            Logins:{' '}
            <strong className="font-semibold text-ink">{logins.label}</strong>
          </li>
          <li className="flex items-center gap-1.5">
            <Shield className="size-3 text-ink-3" />
            Needs OK: {verdicts.ask} · Blocked: {verdicts.block}
          </li>
          <li className="flex items-center gap-1.5">
            <Globe className="size-3 text-ink-3" />
            {aclCount} ACL rule{aclCount === 1 ? '' : 's'}
          </li>
        </ul>
      </div>

      <span className="font-bold text-[11px] text-ink-4 uppercase tracking-wider">
        MCP endpoint
      </span>
      <div className="flex items-center gap-2 rounded-lg bg-[#15140F] px-3 py-2">
        <code className="min-w-0 flex-1 truncate font-mono text-[#EDEAE2] text-[11px]">
          {mcpUrl}
        </code>
        <button
          type="button"
          onClick={copyMcpUrl}
          className="flex size-6 items-center justify-center rounded-md bg-white/10 text-white hover:bg-white/20"
          aria-label="Copy MCP URL"
        >
          {copied ? <Check className="size-3" /> : <Copy className="size-3" />}
        </button>
      </div>

      <div className="flex-1" />

      {submitted ? (
        <div className="flex items-center gap-2 rounded-xl border border-[#BFE3CC] bg-green-tint p-3">
          <CheckCircle2 className="size-5 text-green" />
          <div className="min-w-0">
            <div className="font-bold text-[#15683A] text-sm">
              {successHeadline}
            </div>
            <div className="text-ink-2 text-xs">{successDetail}</div>
          </div>
        </div>
      ) : (
        <Button
          type="submit"
          size="lg"
          disabled={nameInvalid || isMutating}
          className="w-full"
        >
          <Link2 className="size-4" />
          {isMutating ? pendingCtaLabel : idleCtaLabel}
        </Button>
      )}

      <p className="text-center text-[10.5px] text-ink-4">
        {isEdit
          ? 'CLI:'
          : `One click registers the endpoint with ${values.harness}. CLI:`}{' '}
        <span className="font-mono">{cliCommand}</span>
      </p>

      {submitted && (
        <Button
          type="button"
          variant="ghost"
          onClick={onDone}
          className="w-full"
        >
          Done
        </Button>
      )}
    </aside>
  )
}
