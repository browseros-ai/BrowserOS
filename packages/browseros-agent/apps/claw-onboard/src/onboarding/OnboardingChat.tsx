/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useEffect, useReducer, useRef, useState } from 'react'
import { flushSync } from 'react-dom'
import { cn } from '@/lib/utils'
import {
  BROWSEROS_ONBOARDING_API_VERSION,
  type BrowserOSImportItem,
  type BrowserOSOnboardingState,
  type BrowserOSStartImportRequest,
} from './browseros-onboarding-api'
import {
  type BrowserOSOnboardingBridge,
  createBrowserOSOnboardingBridge,
} from './browseros-onboarding-bridge'
import { ChatThread } from './components/ChatThread'
import { ReplyDock } from './components/ReplyDock'
import { detectPlatform } from './import-selection'
import {
  buildOnboardingChat,
  type ChatAction,
  chatChoicesReducer,
  chatImportRequest,
  chatSource,
  initialChatChoices,
} from './onboarding-chat'
import { useChatReveal } from './use-chat-reveal'

const initialOnboardingState: BrowserOSOnboardingState = {
  apiVersion: BROWSEROS_ONBOARDING_API_VERSION,
  status: 'idle',
  sources: [],
}

/**
 * The import click. The person's reply and the Mac heads-up are committed to
 * the DOM first (flushSync), so the heads-up is on screen before macOS can
 * raise the Keychain prompt; then the request goes out on the same click
 * stack, because Chromium rejects an import that is not a recent interaction.
 */
export function startImportFromClick(
  request: BrowserOSStartImportRequest,
  bridge: Pick<BrowserOSOnboardingBridge, 'startImport'>,
  commitReply: () => void,
) {
  flushSync(commitReply)
  bridge.startImport(request)
}

/** Long enough to read neo's "Done." line before the chat hands off. */
const HANDOFF_AFTER_IMPORT_MS = 1500

/**
 * The first-run WebUI page. It talks to the browser only through the bridge:
 * Chromium snapshots arrive via `receiveState`, and every message goes out
 * from a reply click. The page ends by asking native to COMPLETE (after the
 * import, or on Not now / Reconnect); native prepares the product, then
 * navigates to the cockpit, where neo's chat carries on.
 */
export function OnboardingChat() {
  const [bridge] = useState(createBrowserOSOnboardingBridge)
  const [state, setState] = useState(initialOnboardingState)
  const [choices, dispatch] = useReducer(chatChoicesReducer, initialChatChoices)
  const [platform] = useState(detectPlatform)
  const didNotifyPageReady = useRef(false)
  const input = { state, choices, platform }
  const chat = buildOnboardingChat(input)
  const reveal = useChatReveal(chat.turns)

  useEffect(() => {
    // Install first: pageReady may synchronously restore an in-flight setup
    // after reload. Receiving it only renders state; it never sends COMPLETE.
    const cleanup = bridge.registerReceiver(setState)
    if (!didNotifyPageReady.current) {
      didNotifyPageReady.current = true
      bridge.pageReady()
    }
    return cleanup
  }, [bridge])

  const importSucceeded =
    choices.importRequested && state.status === 'succeeded'
  useEffect(() => {
    if (!importSucceeded) return
    // COMPLETE needs no gesture; the bridge latches it, so a repeat is a no-op.
    const timer = window.setTimeout(
      () => bridge.complete(),
      HANDOFF_AFTER_IMPORT_MS,
    )
    return () => window.clearTimeout(timer)
  }, [bridge, importSucceeded])

  function onAction(action: ChatAction, label: string) {
    switch (action) {
      case 'start':
        dispatch({ type: 'start' })
        return
      case 'reconnect':
      case 'skip':
        dispatch({ type: 'exit', reply: label })
        bridge.complete()
        return
      case 'import': {
        const request = chatImportRequest(input)
        if (request) {
          startImportFromClick(request, bridge, () =>
            dispatch({ type: 'importRequested' }),
          )
        }
        return
      }
      case 'refresh':
        bridge.refreshSources()
        return
      case 'retrySetup':
        bridge.retrySetup()
        return
    }
  }

  function onToggleItem(item: BrowserOSImportItem) {
    const source = chatSource(input)
    if (source) dispatch({ type: 'toggleItem', source, item })
  }

  return (
    <div className="flex h-screen justify-center overflow-hidden bg-bg-canvas text-ink-deep">
      <ProgressDashes lit={chat.progress} />
      <main className="flex h-full w-[640px] max-w-[calc(100vw-32px)] flex-col pt-[100px] pb-[76px]">
        <div className="my-auto flex max-h-[700px] min-h-0 flex-auto flex-col">
          <ChatThread
            turns={chat.turns}
            reveal={reveal}
            onToggleItem={onToggleItem}
          />
          <ReplyDock
            items={chat.dock}
            visible={reveal.settled}
            onAction={onAction}
            onPickSource={(sourceId) =>
              dispatch({ type: 'pickSource', sourceId })
            }
          />
        </div>
      </main>
    </div>
  )
}

function ProgressDashes({ lit }: { lit: number }) {
  return (
    <div
      role="progressbar"
      aria-label="Onboarding progress"
      aria-valuemin={1}
      aria-valuemax={3}
      aria-valuenow={lit}
      className="absolute top-[30px] left-1/2 flex -translate-x-1/2 gap-1.5"
    >
      {[1, 2, 3].map((dash) => (
        <i
          key={dash}
          className={cn(
            'h-1 w-9 rounded-sm transition-colors duration-500',
            dash <= lit ? 'bg-ink-deep' : 'bg-accent-tint-2',
          )}
        />
      ))}
    </div>
  )
}
