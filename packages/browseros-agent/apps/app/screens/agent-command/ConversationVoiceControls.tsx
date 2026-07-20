import { AudioLines, Loader2, Mic, Square } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'

export interface VoicePresentationInput {
  enabled: boolean
  isRecording: boolean
  isTranscribing: boolean
  error: string | null
}

export function resolveVoicePresentation({
  enabled,
  isRecording,
  isTranscribing,
  error,
}: VoicePresentationInput): Omit<VoicePresentationInput, 'enabled'> {
  if (!enabled) {
    return {
      isRecording: false,
      isTranscribing: false,
      error: null,
    }
  }

  return { isRecording, isTranscribing, error }
}

export interface ConversationVoiceControlsProps {
  enabled: boolean
  isRecording: boolean
  isTranscribing: boolean
  onStartRecording: () => void
  onStopRecording: () => void
  onOpenVoiceMode?: () => void
}

export const ConversationVoiceControls: FC<ConversationVoiceControlsProps> = ({
  enabled,
  isRecording,
  isTranscribing,
  onStartRecording,
  onStopRecording,
  onOpenVoiceMode,
}) => {
  if (!enabled) return null

  return (
    <>
      {onOpenVoiceMode ? (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          onClick={onOpenVoiceMode}
          className="h-10 w-10 flex-shrink-0 rounded-xl text-muted-foreground transition-colors hover:text-foreground"
          title="Open voice mode"
          aria-label="Open voice mode"
        >
          <AudioLines className="h-5 w-5" />
        </Button>
      ) : null}

      {isRecording ? (
        <Button
          type="button"
          size="icon"
          onClick={onStopRecording}
          className="h-10 w-10 flex-shrink-0 rounded-xl bg-red-600 text-white hover:bg-red-700"
          title="Stop voice input"
          aria-label="Stop voice input"
        >
          <Square className="h-4 w-4" />
        </Button>
      ) : isTranscribing ? (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          disabled
          className="h-10 w-10 flex-shrink-0 rounded-xl"
          title="Transcribing voice input"
          aria-label="Transcribing voice input"
        >
          <Loader2 className="h-5 w-5 animate-spin" />
        </Button>
      ) : (
        <Button
          type="button"
          variant="ghost"
          size="icon"
          onClick={onStartRecording}
          className="h-10 w-10 flex-shrink-0 rounded-xl text-muted-foreground transition-colors hover:text-foreground"
          title="Voice input"
          aria-label="Voice input"
        >
          <Mic className="h-5 w-5" />
        </Button>
      )}
    </>
  )
}
