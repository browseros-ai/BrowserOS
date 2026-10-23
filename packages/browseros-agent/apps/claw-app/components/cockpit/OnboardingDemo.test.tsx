/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { OnboardingDemo, setStageMotionPaused } from './OnboardingDemo'

function fakeAnimation(playState: AnimationPlayState) {
  const calls: string[] = []
  return {
    calls,
    animation: {
      playState,
      currentTime: 420,
      pause: () => calls.push('pause'),
      play: () => calls.push('play'),
    } as unknown as Animation,
  }
}

function stageWith(animations: Animation[]) {
  const asked: unknown[] = []
  return {
    asked,
    stage: {
      getAnimations: (options?: GetAnimationsOptions) => {
        asked.push(options)
        return animations
      },
    },
  }
}

describe('setStageMotionPaused', () => {
  it('freezes every running transition and animation in the stage subtree', () => {
    const pointer = fakeAnimation('running')
    const halo = fakeAnimation('running')
    const { stage, asked } = stageWith([pointer.animation, halo.animation])
    setStageMotionPaused(stage, true)
    expect(asked).toEqual([{ subtree: true }])
    expect(pointer.calls).toEqual(['pause'])
    expect(halo.calls).toEqual(['pause'])
    // Held at the moment of pausing, not wherever the next frame lands.
    expect(pointer.animation.currentTime).toBe(420)
  })

  it('resumes only what it froze, never replaying a finished animation', () => {
    const frozen = fakeAnimation('paused')
    const finished = fakeAnimation('finished')
    const { stage } = stageWith([frozen.animation, finished.animation])
    setStageMotionPaused(stage, false)
    expect(frozen.calls).toEqual(['play'])
    expect(finished.calls).toEqual([])
  })

  it('tolerates a stage that is not mounted yet', () => {
    expect(() => setStageMotionPaused(null, true)).not.toThrow()
  })
})

describe('OnboardingDemo', () => {
  it('describes the scene in text and keeps the drawing out of the a11y tree', () => {
    const html = renderToStaticMarkup(<OnboardingDemo />)
    expect(html).not.toContain('role="img"')
    expect(html).toContain(
      'class="sr-only">Demo on a Mac: you open the Claude app',
    )
    expect(html).toContain('aria-label="Pause the demo"')
    expect(html).toContain('aria-label="Replay the demo"')
    // Final Paper cut the caption; the tiles show their names on hover.
    expect(html).not.toContain('to see it there')
    expect(html).not.toContain('<figcaption')
  })
})
