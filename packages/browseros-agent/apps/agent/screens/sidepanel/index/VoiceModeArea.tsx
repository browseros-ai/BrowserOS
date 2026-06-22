import { useSelector } from '@xstate/store/react'
import type { ReactNode } from 'react'
import { cn } from '@/lib/utils'
import type { VoiceLoopApi } from '@/modules/voice/voice-types'
import { VoiceMode } from './VoiceMode'

interface VoiceModeAreaProps {
  voiceLoop?: VoiceLoopApi
  children: ReactNode
}

// Subscribes to the voice loop's `state` slice directly so the
// parent (ChatFooter) does not have to re-render every time voice
// state changes. ChatFooter re-renders many times per second from
// chat-token streaming; pushing the subscription down to this
// small wrapper keeps the rest of the input row stable.
export function VoiceModeArea({ voiceLoop, children }: VoiceModeAreaProps) {
  if (!voiceLoop) {
    return (
      <div className="relative min-h-[2.625rem] transition-[min-height] duration-200">
        {children}
      </div>
    )
  }
  return <ActiveArea voiceLoop={voiceLoop}>{children}</ActiveArea>
}

function ActiveArea({
  voiceLoop,
  children,
}: {
  voiceLoop: VoiceLoopApi
  children: ReactNode
}) {
  const state = useSelector(voiceLoop.store, (s) => s.context.state)
  const voiceActive = state !== 'idle' && state !== 'closed'

  return (
    <div
      className={cn(
        'relative transition-[min-height] duration-200',
        voiceActive ? 'min-h-[15rem]' : 'min-h-[2.625rem]',
      )}
    >
      <div className={cn(voiceActive && 'invisible')}>{children}</div>
      {voiceActive && <VoiceMode api={voiceLoop} />}
    </div>
  )
}
