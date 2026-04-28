import { EXTERNAL_URLS } from '@browseros/shared/constants/urls'

interface ReferralResult {
  success: boolean
  creditsAdded?: number
  reason?: string
}

export async function submitReferral(
  tweetUrl: string,
  browserosId: string,
): Promise<ReferralResult> {
  const response = await fetch(
    `${EXTERNAL_URLS.REFERRAL_SERVICE}/referral/submit`,
    {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ tweetUrl, browserosId }),
    },
  )
  if (!response.ok) {
    return {
      success: false,
      reason: `Request failed with status ${response.status}`,
    }
  }
  return response.json()
}

const TWEET_VARIATIONS = [
  `ngl @browseros_ai is kinda wild

just type what u want in plain english and it handles the annoying web shit

forms, research, data pulls... all automated

actually works`,

  `been using @browseros_ai to chat with webpages lately

summarize articles, pull data, translate stuff

all happens in the same tab

no copy/paste, no switching windows

just ask and it does it`,

  `wake up to @browseros_ai having already read ur emails and calendar while u were sleeping

scheduled agents are lowkey magic`,

  `ngl @browseros_ai is kinda crazy

connects gmail, slack, linear, notion + 40 other apps into one ai assistant

just talk to it in plain english and it handles cross-app workflows for u

no more switching between tabs like a psycho`,

  `i use @browseros_ai to automate research

it handles the browser work and drops reports straight into local folders

no switching between tools or manually saving files

just one task instead of three`,

  `been messing with @browseros_ai lately

it comes with a prebuilt MCP server and I connect it claude code or codex and it just runs things for you

set it up once, use it whenever

way better than clicking through the same shit manually every time`,

  `the ai actually remembers what we talked about yesterday

no more "here's the context again" every single conversation

@browseros_ai just picks up where we left off

feels like talking to someone who actually pays attention`,

  `i built a skill library for my ai agent

now when i need it to do something specific, i just load the recipe i made earlier

@browseros_ai MCP is very handy`,

  `been running @browseros_ai with ollama locally

everything stays on my machine, nothing gets sent out

kinda nice not having to think about what data i'm sharing`,

  `switched to @browseros_ai from chrome

blocks 10x more ads and runs full ublock origin (not the lite version)

check it out`,
]

export function getShareOnTwitterUrl(): string {
  const text =
    TWEET_VARIATIONS[Math.floor(Math.random() * TWEET_VARIATIONS.length)]
  return `https://x.com/intent/tweet?text=${encodeURIComponent(text)}`
}
