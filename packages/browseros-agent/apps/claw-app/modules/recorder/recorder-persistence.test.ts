import { describe, expect, it } from 'bun:test'
import { createRecorderPersistence } from './recorder-persistence'

describe('recorder persistence acknowledgment', () => {
  it('remembers a failure that settled before the stop request', async () => {
    const persistence = createRecorderPersistence()
    persistence.track(Promise.resolve(false))
    await Promise.resolve()
    persistence.track(Promise.resolve(true))
    await Promise.resolve()
    expect(await persistence.confirmed()).toBe(false)
  })

  it('waits for an in-flight write before acknowledging stop', async () => {
    const persistence = createRecorderPersistence()
    let finish: (persisted: boolean) => void = () => {}
    persistence.track(
      new Promise<boolean>((resolve) => {
        finish = resolve
      }),
    )
    let acknowledged = false
    const confirmation = persistence.confirmed().then((result) => {
      acknowledged = true
      return result
    })
    await Promise.resolve()
    expect(acknowledged).toBe(false)
    finish(true)
    expect(await confirmation).toBe(true)
  })
})
