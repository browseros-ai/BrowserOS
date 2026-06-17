/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { ExecutorRunGoneError, StubBrowserExecutor } from '../../src/executor'

function makeStartInput() {
  return {
    agentId: 'agent-1',
    task: 'file expenses',
    site: 'concur.com',
  }
}

describe('StubBrowserExecutor', () => {
  test('startRun returns a handle with a nanoid id and the input fields', async () => {
    const exec = new StubBrowserExecutor()
    const handle = await exec.startRun(makeStartInput())
    expect(handle.id).toMatch(/^[A-Za-z0-9_-]+$/)
    expect(handle.agentId).toBe('agent-1')
    expect(handle.task).toBe('file expenses')
    expect(handle.site).toBe('concur.com')
  })

  test('per-tool dispatch returns deterministic observations', async () => {
    const exec = new StubBrowserExecutor()
    const handle = await exec.startRun(makeStartInput())

    const nav = await exec.navigate(handle, { url: 'https://docs.google.com' })
    expect(nav).toEqual({
      verb: 'navigate',
      ok: true,
      summary: '(stub) navigated to https://docs.google.com',
      detail: {
        url: 'https://docs.google.com',
        title: 'stub page',
        status: 200,
      },
    })

    const read = await exec.read(handle, { selector: '#main' })
    expect(read.summary).toBe('(stub) read #main')
    expect(read.detail).toEqual({ selector: '#main', text: 'lorem ipsum' })

    const readDefault = await exec.read(handle, {})
    expect(readDefault.summary).toBe('(stub) read document')

    const click = await exec.click(handle, { selector: '.btn' })
    expect(click.summary).toBe('(stub) clicked .btn')

    const typed = await exec.type(handle, { selector: '#q', value: 'hi' })
    expect(typed.summary).toBe('(stub) typed hi into #q')

    const attached = await exec.attach(handle, {
      selector: '#file',
      filePath: '/tmp/receipt.pdf',
    })
    expect(attached.summary).toBe('(stub) attached receipt.pdf to #file')
    expect(attached.detail).toEqual({ selector: '#file', file: 'receipt.pdf' })

    const submitted = await exec.submit(handle, { selector: 'form#expenses' })
    expect(submitted.summary).toBe('(stub) submitted form#expenses')
  })

  test('stop is idempotent and a second stop on an unknown run is a no-op', async () => {
    const exec = new StubBrowserExecutor()
    const handle = await exec.startRun(makeStartInput())
    await exec.stop(handle)
    // Second stop must not throw.
    await exec.stop(handle)
    // Stopping a never-seen handle must not throw either.
    await exec.stop({ id: 'ghost', agentId: 'x', task: 't', site: 's' })
  })

  test('dispatch after stop throws ExecutorRunGoneError carrying the run id', async () => {
    const exec = new StubBrowserExecutor()
    const handle = await exec.startRun(makeStartInput())
    await exec.stop(handle)
    expect(
      exec.navigate(handle, { url: 'https://example.com' }),
    ).rejects.toBeInstanceOf(ExecutorRunGoneError)
    try {
      await exec.click(handle, { selector: '.btn' })
      throw new Error('unreachable')
    } catch (err) {
      expect(err).toBeInstanceOf(ExecutorRunGoneError)
      expect((err as ExecutorRunGoneError).runId).toBe(handle.id)
    }
  })

  test('dispatch against a never-started run throws ExecutorRunGoneError', async () => {
    const exec = new StubBrowserExecutor()
    expect(
      exec.click(
        { id: 'ghost', agentId: 'x', task: 't', site: 's' },
        { selector: '.btn' },
      ),
    ).rejects.toBeInstanceOf(ExecutorRunGoneError)
  })

  test('two concurrent runs are isolated: stopping one does not affect the other', async () => {
    const exec = new StubBrowserExecutor()
    const a = await exec.startRun(makeStartInput())
    const b = await exec.startRun({
      ...makeStartInput(),
      agentId: 'agent-2',
    })
    expect(a.id).not.toBe(b.id)
    await exec.stop(a)
    // a is dead.
    expect(exec.click(a, { selector: '.x' })).rejects.toBeInstanceOf(
      ExecutorRunGoneError,
    )
    // b is still alive.
    const obs = await exec.click(b, { selector: '.y' })
    expect(obs.summary).toBe('(stub) clicked .y')
  })

  test('pause + resume keep the run usable; stop invalidates it', async () => {
    const exec = new StubBrowserExecutor()
    const handle = await exec.startRun(makeStartInput())
    await exec.pause(handle)
    // Paused runs still accept dispatch (paused == "the user clicked
    // pause in the cockpit"; the stub keeps the contract that the run
    // handle is alive until stop).
    const click = await exec.click(handle, { selector: '.btn' })
    expect(click.ok).toBe(true)
    await exec.resume(handle)
    const submit = await exec.submit(handle, { selector: 'form' })
    expect(submit.ok).toBe(true)
    await exec.stop(handle)
    expect(exec.submit(handle, { selector: 'form' })).rejects.toBeInstanceOf(
      ExecutorRunGoneError,
    )
  })
})
