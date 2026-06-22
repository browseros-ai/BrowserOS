export interface AudioLevelSample {
  levels: number[]
  aggregate: number
}

export type AudioLevelListener = (sample: AudioLevelSample) => void

export interface AudioLevelMonitorOptions {
  bandCount?: number
}

export interface AudioLevelMonitor {
  start(analyser: AnalyserNode): void
  stop(): void
  subscribe(listener: AudioLevelListener): () => void
  readonly isRunning: boolean
}

const DEFAULT_BAND_COUNT = 5

export function createAudioLevelMonitor(
  opts: AudioLevelMonitorOptions = {},
): AudioLevelMonitor {
  const bandCount = opts.bandCount ?? DEFAULT_BAND_COUNT
  const listeners = new Set<AudioLevelListener>()
  let currentAnalyser: AnalyserNode | null = null
  let frameHandle: number | null = null

  const tick = () => {
    const analyser = currentAnalyser
    if (!analyser) return
    const dataArray = new Uint8Array(analyser.fftSize)
    analyser.getByteTimeDomainData(dataArray)
    const sample = bandsFrom(dataArray, bandCount)
    for (const fn of listeners) fn(sample)
    frameHandle = requestAnimationFrame(tick)
  }

  return {
    get isRunning() {
      return frameHandle !== null
    },
    start(analyser) {
      if (frameHandle !== null) return
      currentAnalyser = analyser
      frameHandle = requestAnimationFrame(tick)
    },
    stop() {
      if (frameHandle !== null) cancelAnimationFrame(frameHandle)
      frameHandle = null
      currentAnalyser = null
      for (const fn of listeners) fn(emptySample(bandCount))
    },
    subscribe(listener) {
      listeners.add(listener)
      return () => {
        listeners.delete(listener)
      }
    },
  }
}

function bandsFrom(dataArray: Uint8Array, bandCount: number): AudioLevelSample {
  const binCount = dataArray.length
  const levels: number[] = []
  let totalPeak = 0
  for (let band = 0; band < bandCount; band++) {
    const start = Math.floor((band / bandCount) * binCount)
    const end = Math.floor(((band + 1) / bandCount) * binCount)
    let peak = 0
    for (let j = start; j < end; j++) {
      const amplitude = Math.abs(dataArray[j] - 128)
      if (amplitude > peak) peak = amplitude
    }
    const normalized = Math.round(Math.min(100, (peak / 50) * 100))
    levels.push(normalized)
    totalPeak += normalized
  }
  return { levels, aggregate: Math.round(totalPeak / bandCount) }
}

export function emptySample(bandCount = DEFAULT_BAND_COUNT): AudioLevelSample {
  return { levels: Array(bandCount).fill(0), aggregate: 0 }
}
