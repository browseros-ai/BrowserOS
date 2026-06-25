import { describe, expect, test } from 'bun:test'
import { RefMap } from './refs'

describe('RefMap', () => {
  test('reuses refs for the same document backend node across snapshots', () => {
    const refs = new RefMap()

    const first = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 1,
      role: 'button',
      name: 'A',
    })
    const second = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 2,
      role: 'link',
      name: 'B',
    })

    refs.beginSnapshot()

    const inserted = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 3,
      role: 'button',
      name: 'X',
    })
    const firstAgain = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 1,
      role: 'button',
      name: 'A',
    })
    const secondAgain = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 2,
      role: 'link',
      name: 'B',
    })

    expect(firstAgain).toBe(first)
    expect(secondAgain).toBe(second)
    expect(inserted).toBe('e3')
    expect([...refs.byRef.keys()]).toEqual(['e3', 'e1', 'e2'])
  })

  test('keeps latest-snapshot refs while preserving stable assignments', () => {
    const refs = new RefMap()

    refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 1,
      role: 'button',
      name: 'A',
    })
    refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 2,
      role: 'button',
      name: 'B',
    })

    refs.beginSnapshot()
    const kept = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 2,
      role: 'button',
      name: 'B',
    })

    expect(kept).toBe('e2')
    expect(refs.get('e1')).toBeUndefined()
    expect(refs.get('e2')).toMatchObject({ backendNodeId: 2 })
  })

  test('resets the public namespace for a new document', () => {
    const refs = new RefMap()

    expect(
      refs.mint({
        documentId: 'main:loader-1',
        backendNodeId: 10,
        role: 'button',
        name: 'Old',
      }),
    ).toBe('e1')
    refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 11,
      role: 'button',
      name: 'Second',
    })

    refs.reset()

    expect(
      refs.mint({
        documentId: 'main:loader-2',
        backendNodeId: 20,
        role: 'button',
        name: 'New',
      }),
    ).toBe('e1')
    expect(refs.size).toBe(1)
  })

  test('uses capture-local traversal order when document identity is unavailable', () => {
    const refs = new RefMap()

    expect(
      refs.mint({ backendNodeId: 1, role: 'button', name: 'Fallback' }),
    ).toBe('e1')

    refs.beginSnapshot()

    expect(
      refs.mint({ backendNodeId: 2, role: 'button', name: 'Fallback' }),
    ).toBe('e1')
  })

  test('scopes backend node identity by frame document', () => {
    const refs = new RefMap()

    const main = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 7,
      role: 'button',
      name: 'Submit',
    })
    const child = refs.mint({
      documentId: 'child:loader-1',
      frameId: 'child',
      backendNodeId: 7,
      role: 'button',
      name: 'Submit',
    })

    refs.beginSnapshot()

    expect(
      refs.mint({
        documentId: 'main:loader-1',
        backendNodeId: 7,
        role: 'button',
        name: 'Submit',
      }),
    ).toBe(main)
    expect(
      refs.mint({
        documentId: 'child:loader-1',
        frameId: 'child',
        backendNodeId: 7,
        role: 'button',
        name: 'Submit',
      }),
    ).toBe(child)
    expect(main).not.toBe(child)
  })

  test('recomputes duplicate nth metadata for each snapshot', () => {
    const refs = new RefMap()

    refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 1,
      role: 'button',
      name: 'OK',
    })
    refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 2,
      role: 'button',
      name: 'OK',
    })

    refs.beginSnapshot()

    const second = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 2,
      role: 'button',
      name: 'OK',
    })
    const first = refs.mint({
      documentId: 'main:loader-1',
      backendNodeId: 1,
      role: 'button',
      name: 'OK',
    })

    expect(refs.get(second)?.nth).toBe(0)
    expect(refs.get(first)?.nth).toBe(1)
  })
})
