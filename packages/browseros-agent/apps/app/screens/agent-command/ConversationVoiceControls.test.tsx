import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import type {
  ConversationVoiceControlsProps,
  VoicePresentationInput,
} from './ConversationVoiceControls'

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

mock.module('@/components/ui/button', () => ({
  Button: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: MockButtonProps) =>
    createElement('button', { type: 'button', ...props }, children),
}))

type ResolveVoicePresentation = (input: VoicePresentationInput) => {
  isRecording: boolean
  isTranscribing: boolean
  error: string | null
}

let ConversationVoiceControls: FC<ConversationVoiceControlsProps>
let resolveVoicePresentation: ResolveVoicePresentation

beforeAll(async () => {
  const voiceControls = await import('./ConversationVoiceControls')
  ConversationVoiceControls = voiceControls.ConversationVoiceControls
  resolveVoicePresentation = voiceControls.resolveVoicePresentation
})

const callbacks = {
  onStartRecording: () => {},
  onStopRecording: () => {},
  onOpenVoiceMode: () => {},
}

describe('ConversationVoiceControls', () => {
  it('hides both voice actions when support is disabled', () => {
    const html = renderToStaticMarkup(
      createElement(ConversationVoiceControls, {
        enabled: false,
        isRecording: false,
        isTranscribing: false,
        ...callbacks,
      }),
    )

    expect(html).toBe('')
  })

  it('shows voice mode and dictation when support is enabled', () => {
    const html = renderToStaticMarkup(
      createElement(ConversationVoiceControls, {
        enabled: true,
        isRecording: false,
        isTranscribing: false,
        ...callbacks,
      }),
    )

    expect(html).toContain('aria-label="Open voice mode"')
    expect(html).toContain('aria-label="Voice input"')
  })
})

describe('resolveVoicePresentation', () => {
  it('suppresses voice-derived input state while support is disabled', () => {
    expect(
      resolveVoicePresentation({
        enabled: false,
        isRecording: true,
        isTranscribing: true,
        error: 'Microphone permission denied',
      }),
    ).toEqual({
      isRecording: false,
      isTranscribing: false,
      error: null,
    })
  })

  it('preserves voice-derived input state while support is enabled', () => {
    expect(
      resolveVoicePresentation({
        enabled: true,
        isRecording: true,
        isTranscribing: true,
        error: 'Microphone permission denied',
      }),
    ).toEqual({
      isRecording: true,
      isTranscribing: true,
      error: 'Microphone permission denied',
    })
  })
})
