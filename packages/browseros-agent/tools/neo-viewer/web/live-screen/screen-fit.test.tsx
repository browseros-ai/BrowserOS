import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { useRef } from 'react'
import { installDom, type TestDom } from '../test-support/dom'
import {
  DEFAULT_SCREEN_RATIO,
  fitScreen,
  MIN_SCREEN_HEIGHT,
  screenRatio,
  useScreenFit,
} from './screen-fit'

describe('the docked screen’s height, from its width', () => {
  test('follows the width at the screen’s ratio, so the picture fills the width', () => {
    // Neo's default 1600×1000 screen in the default sidebar, and dragged wide.
    expect(fitScreen(338, 1.6, 800)).toEqual({ height: 212, capped: false })
    expect(fitScreen(1068, 1.6, 800)).toEqual({ height: 668, capped: false })
    // Another screen size, another ratio.
    expect(fitScreen(960, 1920 / 1080, 800)).toEqual({
      height: 540,
      capped: false,
    })
    expect(fitScreen(1068, 1.6, null)).toEqual({ height: 668, capped: false })
  })

  test('rounds up, so the width sets the picture’s scale and no side bars appear', () => {
    for (let width = 268; width <= 1200; width += 7) {
      const { height } = fitScreen(width, 1.6, null)
      expect(height * 1.6).toBeGreaterThanOrEqual(width)
      expect(height - width / 1.6).toBeLessThan(1)
    }
  })

  test('is capped at the visible height, and only then letterboxes', () => {
    expect(fitScreen(1068, 1.6, 520)).toEqual({ height: 520, capped: true })
    expect(fitScreen(1068, 1.6, 520.7)).toEqual({ height: 520, capped: true })
    // Exactly at the cap is not capped.
    expect(fitScreen(1068, 1.6, 668)).toEqual({ height: 668, capped: false })
    // A very short window: the picture stays readable and the sidebar scrolls.
    expect(fitScreen(1068, 1.6, 40)).toEqual({
      height: MIN_SCREEN_HEIGHT,
      capped: true,
    })
  })

  test('takes the ratio from the framebuffer the server reported, and a default before it', () => {
    expect(screenRatio({ width: 1600, height: 1000 })).toBe(1.6)
    expect(screenRatio({ width: 1280, height: 1024 })).toBe(1.25)
    expect(screenRatio(null)).toBe(DEFAULT_SCREEN_RATIO)
    expect(screenRatio({ width: 0, height: 0 })).toBe(DEFAULT_SCREEN_RATIO)
  })
})

describe('the height kept current as the sidebar is dragged', () => {
  let dom: TestDom
  // linkedom has no layout: the test plays the browser's ResizeObserver.
  let resize: (width: number) => void
  beforeEach(async () => {
    dom = await installDom()
    resize = () => {}
    Object.assign(globalThis, {
      ResizeObserver: class {
        constructor(callback: ResizeObserverCallback) {
          resize = (width) =>
            callback(
              [{ contentRect: { width } } as ResizeObserverEntry],
              this as unknown as ResizeObserver,
            )
        }
        observe() {}
        disconnect() {}
      },
    })
  })
  afterEach(() => dom.restore())

  function Box({ ratio, view }: { ratio: number; view: number }) {
    const box = useRef<HTMLDivElement>(null)
    const fit = useScreenFit(box, ratio, view, () => 30)
    return (
      <div ref={box}>
        {fit
          ? `${fit.height} ${fit.capped ? 'capped' : 'fills'}`
          : 'unmeasured'}
      </div>
    )
  }

  test('each new width gives a new height, capped by the view less what the panel reserves', async () => {
    await dom.render(<Box ratio={1.6} view={700} />)
    await dom.interact(() => resize(338))
    expect(dom.container.textContent).toBe('212 fills')
    await dom.interact(() => resize(1068))
    // The cap is 700 − 30 = 670: 668 still fits, 750 does not.
    expect(dom.container.textContent).toBe('668 fills')
    await dom.interact(() => resize(1200))
    expect(dom.container.textContent).toBe('670 capped')
    // A taller window lifts the cap.
    await dom.render(<Box ratio={1.6} view={1000} />)
    expect(dom.container.textContent).toBe('750 fills')
  })
})
