import { Loader2, Mic, Square } from 'lucide-react'
import type { FC } from 'react'
import { Button } from '@/components/ui/button'

export interface Props {
  isRecording: boolean
  isTranscribing: boolean
  audioLevel: number
  disabled?: boolean
  onStart: () => void
  onStop: () => void
}

export const VoiceInputButton: FC<Props> = ({
  isRecording,
  isTranscribing,
  audioLevel,
  disabled,
  onStart,
  onStop,
}) => {
  if (isTranscribing) {
    return (
      <Button variant="outline" size="icon" disabled>
        <Loader2 className="h-4 w-4 animate-spin" />
      </Button>
    )
  }

  if (isRecording) {
    return (
      <Button
        type="button"
        variant="destructive"
        size="icon"
        onClick={onStop}
        className="relative"
      >
        <Square className="h-4 w-4" />
        <span
          className="absolute inset-0 animate-ping rounded-md bg-destructive/50"
          style={{ opacity: Math.min(0.7, audioLevel / 100) }}
        />
      </Button>
    )
  }

  return (
    <Button type="button" size="icon" onClick={onStart} disabled={disabled}>
      <Mic className="h-4 w-4" />
    </Button>
  )
}
