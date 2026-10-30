import { describe, expect, it } from 'bun:test'
import { ApiResponseError } from '@browseros/claw-api-client'
import type { ToastInput } from '@/components/motion/animated-toast-stack'
import {
  connectToastHost,
  TOAST_DURATION_MS,
  toast,
  toastApiError,
  toastPromise,
  toastSuccess,
} from './toast'

function fakeHost() {
  const shown: ToastInput[] = []
  const updates: { id: string; patch: Partial<ToastInput> }[] = []
  const handle = {
    show: (input: ToastInput) => {
      shown.push(input)
      return input.id ?? String(shown.length)
    },
    update: (id: string, patch: Partial<ToastInput>) => {
      updates.push({ id, patch })
    },
  }
  return { shown, updates, handle }
}

describe('toast helpers', () => {
  it('keeps toasts raised before the host mounts and shows them on connect', () => {
    toast('Saved early')
    const host = fakeHost()
    const disconnect = connectToastHost(host.handle)
    expect(host.shown).toEqual([{ title: 'Saved early' }])
    disconnect()
  })

  it('shows success and error toasts with their status', () => {
    const host = fakeHost()
    const disconnect = connectToastHost(host.handle)
    toastSuccess('Connected Zed')
    toastApiError('Could not connect', new Error('Port 9200 is busy'))
    toastApiError('Could not connect', 'not an error object')
    expect(host.shown).toEqual([
      { title: 'Connected Zed', description: undefined, status: 'success' },
      {
        title: 'Could not connect',
        description: 'Port 9200 is busy',
        status: 'error',
      },
      { title: 'Could not connect', description: undefined, status: 'error' },
    ])
    disconnect()
  })

  it('prefers the server’s own message over the status line', () => {
    const host = fakeHost()
    const disconnect = connectToastHost(host.handle)
    const response = new Response(null, { status: 409 })
    toastApiError(
      'Couldn’t open the payment page',
      new ApiResponseError(response, {
        code: 'already_subscribed',
        message: 'This computer already has Pro.',
      }),
    )
    toastApiError(
      'Couldn’t open the payment page',
      new ApiResponseError(new Response('busy', { status: 502 }), 'busy'),
    )
    expect(host.shown).toEqual([
      {
        title: 'Couldn’t open the payment page',
        description: 'This computer already has Pro.',
        status: 'error',
      },
      {
        title: 'Couldn’t open the payment page',
        description: 'BrowserOS neo API request failed with status 502',
        status: 'error',
      },
    ])
    disconnect()
  })

  it('turns a loading toast into the result once the work finishes', async () => {
    const host = fakeHost()
    const disconnect = connectToastHost(host.handle)
    const work = Promise.resolve(3)
    toastPromise(work, {
      loading: 'Cleaning up…',
      success: (n) => `Removed ${n} sessions`,
      error: 'Could not clean up',
    })
    await work
    await Promise.resolve()
    const [loading] = host.shown
    expect(loading).toMatchObject({ title: 'Cleaning up…', status: 'loading' })
    expect(host.updates).toEqual([
      {
        id: loading?.id as string,
        patch: {
          title: 'Removed 3 sessions',
          status: 'success',
          duration: TOAST_DURATION_MS,
        },
      },
    ])
    disconnect()
  })

  it('reports a failed promise as an error toast without rejecting', async () => {
    const host = fakeHost()
    const disconnect = connectToastHost(host.handle)
    const work = Promise.reject(new Error('Disk is read only'))
    toastPromise(work, {
      loading: 'Saving…',
      success: 'Saved',
      error: 'Could not save',
    })
    await work.catch(() => {})
    await Promise.resolve()
    expect(host.updates.at(-1)?.patch).toEqual({
      title: 'Could not save',
      description: 'Disk is read only',
      status: 'error',
      duration: TOAST_DURATION_MS,
    })
    disconnect()
  })

  it('queues again after the host disconnects', () => {
    const first = fakeHost()
    connectToastHost(first.handle)()
    toast('While unmounted')
    expect(first.shown).toEqual([])
    const second = fakeHost()
    const disconnect = connectToastHost(second.handle)
    expect(second.shown).toEqual([{ title: 'While unmounted' }])
    disconnect()
  })
})
