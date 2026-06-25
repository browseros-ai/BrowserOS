import {
  ArrowDown,
  ArrowRight,
  BookOpenText,
  Bot,
  Code2,
  FolderOpen,
  LinkIcon,
  Plug,
  SplitSquareHorizontal,
} from 'lucide-react'
import { type FC, useEffect, useState } from 'react'
import DiscordLogo from '@/assets/discord-logo.svg'
import GithubLogo from '@/assets/github-logo.svg'
import SlackLogo from '@/assets/slack-logo.svg'
import { PillIndicator } from '@/components/elements/pill-indicator'
import { Button } from '@/components/ui/button'
import {
  AGENT_MODE_DEMO_URL,
  AGENTIC_CODING_DEMO_URL,
  BROWSER_OS_INTRO_VIDEO_URL,
  COWORK_DEMO_URL,
  MCP_SERVER_DEMO_URL,
  SPLIT_VIEW_GIF_URL,
} from '@/lib/constants/mediaUrls'
import {
  discordUrl,
  docsUrl,
  productRepositoryUrl,
  slackUrl,
} from '@/lib/constants/productUrls'
import { cn } from '@/lib/utils'
import { BentoCard, type Feature } from './BentoCard'
import { VideoFrame } from './VideoFrame'

const features: Feature[] = [
  {
    id: 'agent',
    Icon: Bot,
    tag: 'AI AGENT',
    title: 'Built-in AI Agent',
    description:
      'Describe any task and watch BrowserOS execute it—clicking, typing, and navigating for you.',
    detailedDescription:
      'The BrowserOS Agent turns your words into browser actions. Describe what you need in plain English—fill out this form, extract data from that page, navigate through these steps—and the agent handles the rest. It clicks buttons, types text, navigates between pages, and completes multi-step browser tasks automatically. Everything runs locally on your machine with your own API keys, so your data stays private.',
    highlights: [
      'Multi-tab execution — run agents in multiple tabs simultaneously',
      'Smart navigation — automatically finds and interacts with page elements',
      'Form filling — completes forms with intelligent context understanding',
      'Data extraction — pulls structured data from any webpage',
      'Auto-save sessions — pick up where you left off from the Assistant panel',
    ],
    videoDuration: '2:22',
    gridClass: 'md:col-span-2',
    videoUrl: AGENT_MODE_DEMO_URL,
  },
  {
    id: 'mcp-server',
    Icon: Plug,
    tag: 'MCP',
    title: 'BrowserOS as MCP Server',
    description:
      'Connect Claude Code, Gemini CLI, or any MCP client to control your browser with 31 tools.',
    detailedDescription:
      'BrowserOS includes a built-in MCP server that lets AI coding agents control your browser. Claude Code can open tabs, click elements, fill forms, take screenshots, and read page content—all through natural language commands. Unlike Chrome DevTools MCP which requires debug profiles and separate servers, BrowserOS works out of the box. Just copy the URL from settings and connect.',
    highlights: [
      'One-line setup — run `claude mcp add` with your server URL to connect',
      '31 browser tools — tabs, clicks, typing, screenshots, bookmarks, history',
      'Works everywhere — Claude Code, Gemini CLI, Codex, Claude Desktop',
      'Authenticated access — extract data from logged-in pages like LinkedIn',
    ],
    videoDuration: '1:40',
    gridClass: 'md:col-span-1',
    videoUrl: MCP_SERVER_DEMO_URL,
  },
  {
    id: 'cowork',
    Icon: FolderOpen,
    tag: 'FILES',
    title: 'Cowork',
    description:
      'Give the agent access to local files. Research the web, then save reports to your computer.',
    detailedDescription:
      'Cowork lets the agent read and write files on your computer. Select a folder and the agent can read documents, write reports, and run shell commands—all while browsing the web. Research a topic online and generate an HTML report. Scrape product data and save it as a spreadsheet. The agent is sandboxed to your selected folder and cannot access anything outside it.',
    highlights: [
      'Read & write files — create reports, spreadsheets, and markdown documents',
      'Run shell commands — execute commands within your selected folder',
      'Browser + files — combine web research with local file operations',
      'Sandboxed security — agent can only access the folder you select',
    ],
    gridClass: 'md:col-span-2',
    videoUrl: COWORK_DEMO_URL || undefined,
  },
  {
    id: 'split-view',
    Icon: SplitSquareHorizontal,
    tag: 'CORE',
    title: 'Split-View Mode',
    description:
      'Open ChatGPT, Claude, or Gemini alongside any webpage. Compare responses in the LLM Hub.',
    detailedDescription:
      'Access AI chat on any webpage without switching tabs. Click the Chat button or press Alt+K to open a panel with Claude, ChatGPT, or Gemini right next to your current page. Copy page content, attach screenshots, and get answers in context. Open the LLM Hub (Cmd+Shift+U) to query multiple models simultaneously and compare their responses side-by-side.',
    highlights: [
      'AI on any page — chat panel stays open as you browse',
      'LLM Hub — compare responses from Claude, ChatGPT, and Gemini at once',
      'Quick toggle — Alt+K opens chat, Alt+L switches providers',
      'Copy & screenshot — grab page content or capture screenshots for context',
    ],
    gridClass: 'md:col-span-2',
    gifUrl: SPLIT_VIEW_GIF_URL,
  },
  {
    id: 'agentic-coding',
    Icon: Code2,
    tag: 'DEV',
    title: 'Agentic Coding',
    description:
      'Claude Code tests your web app, reads console errors, and fixes your code in one loop.',
    detailedDescription:
      'The killer workflow for frontend developers. Claude Code connects to BrowserOS, opens your localhost app, clicks through the UI, reads console errors and network failures, then goes back to your codebase to fix the bugs—all in one continuous loop. No more switching between terminal and browser. No more copy-pasting error messages. Just describe the issue and let the agent debug it end-to-end.',
    highlights: [
      'Test & fix loop — Claude navigates your app, finds bugs, and patches them',
      'Console access — read browser console and network errors from your terminal',
      'Screenshot debugging — Claude captures screenshots to understand visual issues',
      'Rapid prototyping — build UIs faster with AI that sees your work',
    ],
    gridClass: 'md:col-span-1',
    videoUrl: AGENTIC_CODING_DEMO_URL || undefined,
  },
]

/**
 * @public
 */
export const FeaturesPage: FC = () => {
  const [mounted, setMounted] = useState(false)

  useEffect(() => {
    setMounted(true)
  }, [])

  const handleStart = async () => {
    const newtabUrl = chrome.runtime.getURL('app.html')
    const [currentTab] = await chrome.tabs.query({
      active: true,
      currentWindow: true,
    })
    await chrome.tabs.create({ url: newtabUrl })
    if (currentTab.id) {
      await chrome.tabs.remove(currentTab.id)
    }
  }

  return (
    <div className="min-h-screen bg-background">
      {/* Hero Section */}
      <section className="relative border-border/40 border-b">
        <div className="mx-auto max-w-6xl px-6 py-16 md:py-20">
          <div className="space-y-8 text-center">
            {/* Header */}
            <div className="space-y-6">
              <PillIndicator
                text="WELCOME"
                className={`transition-all delay-100 duration-700 ${mounted ? 'translate-y-0 opacity-100' : 'translate-y-4 opacity-0'}`}
              />

              <div className="space-y-4">
                <h1
                  className={cn(
                    'font-bold text-4xl leading-tight tracking-tight md:text-5xl',
                    'transition-all delay-200 duration-700 md:text-7xl',
                    mounted ? 'scale-100 opacity-100' : 'scale-95 opacity-0',
                  )}
                >
                  Why Switch to{' '}
                  <span className="text-[var(--accent-orange)]">
                    BrowserOS?
                  </span>
                </h1>
                <p
                  className={cn(
                    'mx-auto max-w-2xl text-lg text-muted-foreground leading-relaxed',
                    'transition-all delay-300 duration-700',
                    mounted
                      ? 'translate-y-0 opacity-100'
                      : 'translate-y-4 opacity-0',
                  )}
                >
                  Watch our launch video to understand the vision of BrowserOS
                  and key features!
                </p>
              </div>
            </div>

            {/* Centered Large Video */}
            <VideoFrame
              title="browseros.com/demo"
              className={cn(
                'transition-all delay-500 duration-700',
                mounted
                  ? 'translate-y-0 opacity-100'
                  : 'translate-y-4 opacity-0',
              )}
            >
              <video
                className="h-full w-full"
                src={BROWSER_OS_INTRO_VIDEO_URL}
                title="BrowserOS MCP Server Demonstration"
                autoPlay
                muted
                loop
                playsInline
                controls
              />
            </VideoFrame>
          </div>
        </div>

        <div
          className={cn(
            'animation-duration-[3s] absolute bottom-0.5 left-1/2 flex -translate-x-1/2 animate-bounce flex-col items-center gap-3',
            'transition-opacity delay-[2000ms] duration-700',
            mounted ? 'opacity-100' : 'opacity-0',
          )}
        >
          <div className="text-center">
            <p className="mb-2 font-medium text-muted-foreground text-xs">
              Scroll for Features
            </p>
            <ArrowDown className="mx-auto h-6 w-6 text-[var(--accent-orange)]" />
          </div>
        </div>
      </section>

      {/* Features Bento Grid */}
      <section className="mx-auto max-w-7xl px-6 py-16 md:py-20">
        <div className="mb-12 space-y-3 text-center">
          <p className="font-semibold text-muted-foreground text-xs uppercase tracking-widest">
            FEATURES
          </p>
          <h2 className="font-bold text-3xl tracking-tight md:text-4xl">
            Explore What&apos;s{' '}
            <span className="text-[var(--accent-orange)]">Possible</span>
          </h2>
          <p className="mx-auto max-w-2xl text-muted-foreground">
            Skim the highlights below, then click any card to see a focused
            walkthrough with video and deeper details.
          </p>
        </div>

        {/* Bento Grid */}
        {mounted && (
          <div className="grid gap-4 md:grid-cols-3">
            {features.map((feature, index) => (
              <BentoCard
                key={feature.id}
                feature={feature}
                mounted={mounted}
                index={index}
              />
            ))}
          </div>
        )}

        <div className="mt-8 text-center">
          <p className="text-muted-foreground text-sm">
            💡 Tip: Click any card to open a focused walkthrough with video
          </p>
        </div>
      </section>

      <section className="mx-auto max-w-7xl border-border/40 border-y px-6 py-16">
        <div className="mx-auto max-w-5xl">
          <div className="mb-8 flex items-center gap-3">
            <LinkIcon className="h-6 w-6 text-[var(--accent-orange)]" />
            <h2 className="font-bold text-3xl">
              Join our community and help us improve{' '}
              <span className="text-[var(--accent-orange)]">BrowserOS!</span>
            </h2>
          </div>

          <div className="grid gap-4 md:grid-cols-2">
            {/* Discord */}
            <a
              href={discordUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="community-card group flex items-start gap-4 rounded-xl border border-border bg-card p-6 transition-all hover:-translate-y-1 hover:border-[var(--accent-orange)]/50 hover:bg-card/80 hover:shadow-[var(--accent-orange)]/5 hover:shadow-lg"
            >
              <div className="flex h-12 w-12 flex-shrink-0 items-center justify-center overflow-hidden rounded-lg transition-all group-hover:scale-110">
                <img
                  src={DiscordLogo}
                  className="h-full w-full"
                  alt="discord-logo"
                />
              </div>
              <div className="space-y-1">
                <h3 className="font-semibold text-lg transition-colors group-hover:text-[var(--accent-orange)]">
                  Join Discord
                </h3>
                <p className="text-muted-foreground text-sm">
                  To suggest features / provide feedback
                </p>
              </div>
            </a>

            {/* Slack */}
            <a
              href={slackUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="community-card group flex items-start gap-4 rounded-xl border border-border bg-card p-6 transition-all hover:-translate-y-1 hover:border-[var(--accent-orange)]/50 hover:bg-card/80 hover:shadow-[var(--accent-orange)]/5 hover:shadow-lg"
            >
              <div className="flex h-12 w-12 flex-shrink-0 items-center justify-center overflow-hidden rounded-lg transition-all group-hover:scale-110">
                <img
                  src={SlackLogo}
                  className="h-full w-full"
                  alt="slack-logo"
                />
              </div>
              <div className="space-y-1">
                <h3 className="font-semibold text-lg transition-colors group-hover:text-[var(--accent-orange)]">
                  Join Slack
                </h3>
                <p className="text-muted-foreground text-sm">
                  To suggest features / provide feedback
                </p>
              </div>
            </a>

            {/* GitHub */}
            <a
              href={productRepositoryUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="community-card group flex items-start gap-4 rounded-xl border border-border bg-card p-6 transition-all hover:-translate-y-1 hover:border-[var(--accent-orange)]/50 hover:bg-card/80 hover:shadow-[var(--accent-orange)]/5 hover:shadow-lg"
            >
              <div className="flex h-12 w-12 flex-shrink-0 items-center justify-center overflow-hidden rounded-lg bg-foreground/10 transition-all group-hover:scale-110 group-hover:bg-foreground/20">
                <img
                  src={GithubLogo}
                  className="h-full w-full"
                  alt="github-logo"
                />
              </div>
              <div className="space-y-1">
                <h3 className="font-semibold text-lg transition-colors group-hover:text-[var(--accent-orange)]">
                  GitHub
                </h3>
                <p className="text-muted-foreground text-sm">
                  Star our repository
                </p>
              </div>
            </a>

            {/* Documentation */}
            <a
              href={docsUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="community-card group flex items-start gap-4 rounded-xl border border-border bg-card p-6 transition-all hover:-translate-y-1 hover:border-[var(--accent-orange)]/50 hover:bg-card/80 hover:shadow-[var(--accent-orange)]/5 hover:shadow-lg"
            >
              <div className="flex h-12 w-12 flex-shrink-0 items-center justify-center rounded-lg bg-[var(--accent-orange)]/10 transition-all group-hover:scale-110 group-hover:bg-[var(--accent-orange)]/20">
                <BookOpenText className="h-6 w-6 text-[var(--accent-orange)]" />
              </div>
              <div className="space-y-1">
                <h3 className="font-semibold text-lg transition-colors group-hover:text-[var(--accent-orange)]">
                  Documentation
                </h3>
                <p className="text-muted-foreground text-sm">Learn more</p>
              </div>
            </a>
          </div>
        </div>
      </section>

      <section className="mx-auto max-w-7xl px-6 pt-16 pb-56">
        <div className="space-y-4 text-center">
          <Button
            onClick={handleStart}
            size="lg"
            className="bg-[var(--accent-orange)] text-white shadow-[var(--accent-orange)]/25 shadow-lg hover:bg-[var(--accent-orange)]/90"
          >
            Start Using BrowserOS
            <ArrowRight className="ml-2 h-4 w-4" />
          </Button>
        </div>
      </section>
    </div>
  )
}
