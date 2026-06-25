import { ArrowRight, Globe, Sparkles } from 'lucide-react'
import { useEffect, useState } from 'react'
import { McpServerIcon } from '@/components/mcp/McpServerIcon'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  ONBOARDING_COMPLETED_EVENT,
  ONBOARDING_DEMO_TRIGGERED_EVENT,
} from '@/lib/constants/analyticsEvents'
import { openSidePanelWithSearch } from '@/lib/messaging/sidepanel/openSidepanelWithSearch'
import { track } from '@/lib/metrics/track'
import {
  onboardingCompletedStorage,
  onboardingProfileStorage,
} from '@/lib/onboarding/onboardingStorage'
import { useGetUserMCPIntegrations } from '@/modules/mcp/user-integrations.hooks'

interface DemoSuggestion {
  label: string
  query: string
  mode: 'chat' | 'agent'
  appName?: string
}

const APP_PROMPTS: Record<string, Omit<DemoSuggestion, 'appName'>[]> = {
  Gmail: [
    {
      label: 'Summarize my unread emails and highlight anything urgent',
      query: 'Summarize my unread emails and highlight anything urgent',
      mode: 'agent',
    },
    {
      label: 'Show the last 5 emails from my manager',
      query:
        'Show the last 5 emails from my manager and list any action items mentioned',
      mode: 'agent',
    },
  ],
  'Google Calendar': [
    {
      label: 'What meetings do I have tomorrow?',
      query:
        "What meetings do I have tomorrow? Who's attending and what's the agenda?",
      mode: 'agent',
    },
    {
      label: 'Show my schedule for this week',
      query: 'Show my schedule for this week and flag any double-bookings',
      mode: 'agent',
    },
  ],
  Notion: [
    {
      label: 'List my recently updated Notion pages',
      query: 'List my recently updated Notion pages and summarize what changed',
      mode: 'agent',
    },
    {
      label: 'Show all Notion tasks assigned to me',
      query: 'Show all Notion tasks assigned to me and their current status',
      mode: 'agent',
    },
  ],
  Slack: [
    {
      label: 'Show my unread Slack mentions',
      query: 'Show my unread Slack mentions and summarize each thread',
      mode: 'agent',
    },
    {
      label: 'Latest messages in my most active Slack channels',
      query: 'What are the latest messages in my most active Slack channels?',
      mode: 'agent',
    },
  ],
  GitHub: [
    {
      label: 'Show my open GitHub issues sorted by priority',
      query: 'Show my open GitHub issues sorted by priority',
      mode: 'agent',
    },
    {
      label: 'List my recent GitHub pull requests',
      query: 'List my recent GitHub pull requests and their review status',
      mode: 'agent',
    },
  ],
  Linear: [
    {
      label: 'What Linear tickets are assigned to me?',
      query:
        'What Linear tickets are assigned to me? Show status and any recent comments',
      mode: 'agent',
    },
    {
      label: 'Show my current Linear sprint progress',
      query:
        'Show my current Linear sprint and how many tickets are completed vs remaining',
      mode: 'agent',
    },
  ],
  Jira: [
    {
      label: 'What Jira tickets are assigned to me?',
      query: 'What Jira tickets are assigned to me? Show status and priority',
      mode: 'agent',
    },
    {
      label: 'Summarize recent comments on my open Jira issues',
      query: 'Summarize recent comments on my open Jira issues',
      mode: 'agent',
    },
  ],
  'Google Docs': [
    {
      label: 'List my recently edited Google Docs',
      query:
        'List my recently edited Google Docs and who else has been editing them',
      mode: 'agent',
    },
    {
      label: 'Show my shared Google Docs with recent comments',
      query: 'Show my shared Google Docs and summarize any recent comments',
      mode: 'agent',
    },
  ],
}

function buildDefaultSuggestions(company?: string): DemoSuggestion[] {
  return [
    company
      ? {
          label: `Search for ${company} and summarize the latest news`,
          query: `Search for ${company} and summarize the latest news about them`,
          mode: 'agent' as const,
        }
      : {
          label: "What's the top tech news today",
          query: "What's the top tech news today? Give me a brief summary",
          mode: 'agent' as const,
        },
    {
      label: "What's the top news today",
      query:
        "What's the top news today? Give me a brief summary of the biggest stories",
      mode: 'agent' as const,
    },
    {
      label: 'Find me a good restaurant nearby',
      query: 'Find me a good restaurant nearby',
      mode: 'agent' as const,
    },
  ]
}

function buildPersonalizedSuggestions(
  connectedApps: string[],
): DemoSuggestion[] {
  const suggestions: DemoSuggestion[] = []
  const usedApps = new Set<string>()

  for (const appName of connectedApps) {
    if (usedApps.has(appName)) continue

    const prompts = APP_PROMPTS[appName]
    if (prompts?.[0]) {
      suggestions.push({ ...prompts[0], appName })
      usedApps.add(appName)
    }
  }

  return suggestions
}

function buildCompanyPrompt(company?: string): DemoSuggestion {
  return company
    ? {
        label: `Search for ${company} and summarize the latest news`,
        query: `Search for ${company} and summarize the latest news about them`,
        mode: 'agent',
      }
    : {
        label: "What's the top tech news today",
        query: "What's the top tech news today? Give me a brief summary",
        mode: 'agent',
      }
}

export const OnboardingDemo = () => {
  const [customQuery, setCustomQuery] = useState('')
  const [demoSuggestions, setDemoSuggestions] = useState<DemoSuggestion[]>(() =>
    buildDefaultSuggestions(),
  )
  const { data: userIntegrations } = useGetUserMCPIntegrations()

  useEffect(() => {
    onboardingProfileStorage.getValue().then((profile) => {
      const connectedApps =
        userIntegrations?.integrations
          ?.filter((i) => i.is_authenticated)
          .map((i) => i.name) ?? []

      const companyPrompt = buildCompanyPrompt(profile?.company)

      if (connectedApps.length > 0) {
        const personalized = buildPersonalizedSuggestions(connectedApps)
        if (personalized.length > 0) {
          setDemoSuggestions([...personalized, companyPrompt])
          return
        }
      }

      setDemoSuggestions(buildDefaultSuggestions(profile?.company))
    })
  }, [userIntegrations])

  const completeOnboarding = async () => {
    await onboardingCompletedStorage.setValue(true)
    track(ONBOARDING_COMPLETED_EVENT)
  }

  const handleDemoTask = async (
    query: string,
    mode: 'chat' | 'agent',
    index: number,
  ) => {
    track(ONBOARDING_DEMO_TRIGGERED_EVENT, {
      query,
      mode,
      source: 'suggestion',
      suggestion_index: index,
    })
    await completeOnboarding()

    await chrome.tabs.create({ active: true })
    await new Promise((resolve) => setTimeout(resolve, 500))
    openSidePanelWithSearch('open', { query, mode })
  }

  const handleCustomQuery = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!customQuery.trim()) return

    track(ONBOARDING_DEMO_TRIGGERED_EVENT, {
      query: customQuery.trim(),
      mode: 'agent',
      source: 'custom',
    })
    await completeOnboarding()

    await chrome.tabs.create({ active: true })
    await new Promise((resolve) => setTimeout(resolve, 500))
    openSidePanelWithSearch('open', {
      query: customQuery.trim(),
      mode: 'agent',
    })
  }

  const handleSkip = async () => {
    track(ONBOARDING_DEMO_TRIGGERED_EVENT, { skipped: true })
    await completeOnboarding()
    window.location.href = chrome.runtime.getURL('app.html#/home')
  }

  return (
    <div className="flex h-screen flex-col items-center justify-center bg-background px-6">
      <div className="w-full max-w-lg space-y-8">
        <div className="space-y-2 text-center">
          <div className="mx-auto mb-4 flex size-12 items-center justify-center rounded-full bg-[var(--accent-orange)]/10">
            <Sparkles className="size-6 text-[var(--accent-orange)]" />
          </div>
          <h2 className="font-bold text-3xl tracking-tight">
            Try your first task
          </h2>
          <p className="text-base text-muted-foreground">
            Pick a suggestion or type your own to see BrowserOS in action
          </p>
        </div>

        <div
          className="styled-scrollbar space-y-3 overflow-y-auto pr-1"
          style={{ maxHeight: 'calc(5 * 56px + 4 * 12px)' }}
        >
          {demoSuggestions.map((suggestion, index) => (
            <button
              key={suggestion.label}
              type="button"
              onClick={() =>
                handleDemoTask(suggestion.query, suggestion.mode, index)
              }
              className="flex w-full items-center gap-3 rounded-lg border border-border bg-card p-4 text-left transition-colors hover:border-[var(--accent-orange)]/50 hover:bg-accent"
            >
              <div className="flex size-8 shrink-0 items-center justify-center rounded-md bg-muted">
                {suggestion.appName ? (
                  <McpServerIcon serverName={suggestion.appName} size={18} />
                ) : (
                  <Globe className="size-[18px] text-muted-foreground" />
                )}
              </div>
              <span className="min-w-0 flex-1 font-medium text-sm">
                {suggestion.label}
              </span>
              <ArrowRight className="size-4 shrink-0 text-muted-foreground" />
            </button>
          ))}
        </div>

        <form onSubmit={handleCustomQuery} className="flex gap-2">
          <Input
            placeholder="Or type your own task..."
            value={customQuery}
            onChange={(e) => setCustomQuery(e.target.value)}
            className="flex-1"
          />
          <Button
            type="submit"
            disabled={!customQuery.trim()}
            className="bg-[var(--accent-orange)] text-white hover:bg-[var(--accent-orange)]/90"
          >
            Go
          </Button>
        </form>

        <div className="text-center">
          <Button
            variant="ghost"
            onClick={handleSkip}
            className="text-muted-foreground"
          >
            Skip and go to homepage
          </Button>
        </div>
      </div>
    </div>
  )
}
