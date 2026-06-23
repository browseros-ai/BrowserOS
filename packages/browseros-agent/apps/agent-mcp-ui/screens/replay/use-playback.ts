import { useEffect, useState } from 'react'
import { PLAYBACK_SPEEDS } from './replay.helpers'

export interface Playback {
  /** Seconds elapsed in the session. */
  time: number
  /** True while the wallclock timer is advancing. */
  isPlaying: boolean
  /** Multiplier applied to the 100ms tick. Always one of `PLAYBACK_SPEEDS`. */
  speed: number
  setSpeed: (next: number) => void
  /** Toggles play/pause. Restarts from 0 if the session already finished. */
  togglePlay: () => void
  /** Jumps the playhead to `seconds` (clamped to [0, totalSeconds]). */
  seek: (seconds: number) => void
}

const TICK_MS = 100

export function usePlayback(totalSeconds: number): Playback {
  const [time, setTime] = useState(0)
  const [isPlaying, setIsPlaying] = useState(true)
  const [speed, setSpeed] = useState<number>(PLAYBACK_SPEEDS[0])

  // External wall-clock timer: starting/cancelling setInterval is one of
  // the cases the project conventions still allow useEffect for.
  useEffect(() => {
    if (!isPlaying || totalSeconds === 0) return
    const id = window.setInterval(() => {
      setTime((prev) => {
        const next = prev + (TICK_MS / 1000) * speed
        if (next >= totalSeconds) {
          setIsPlaying(false)
          return totalSeconds
        }
        return next
      })
    }, TICK_MS)
    return () => window.clearInterval(id)
  }, [isPlaying, speed, totalSeconds])

  const togglePlay = () => {
    if (time >= totalSeconds) {
      setTime(0)
      setIsPlaying(true)
      return
    }
    setIsPlaying((p) => !p)
  }

  const seek = (seconds: number) => {
    const clamped = Math.max(0, Math.min(totalSeconds, seconds))
    setTime(clamped)
  }

  return { time, isPlaying, speed, setSpeed, togglePlay, seek }
}
