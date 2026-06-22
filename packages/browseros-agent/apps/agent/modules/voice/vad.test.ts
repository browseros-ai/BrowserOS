import { describe, expect, it } from 'bun:test'
import { pcmFloat32ToWavBlob } from './vad'

describe('pcmFloat32ToWavBlob', () => {
  it('writes a RIFF header with the correct length fields', async () => {
    const samples = new Float32Array([0, 0.5, -0.5, 0.99, -0.99])
    const blob = pcmFloat32ToWavBlob(samples, 16000)
    expect(blob.type).toBe('audio/wav')
    expect(blob.size).toBe(44 + samples.length * 2)
    const ab = await blob.arrayBuffer()
    const view = new DataView(ab)
    const riff = String.fromCharCode(
      view.getUint8(0),
      view.getUint8(1),
      view.getUint8(2),
      view.getUint8(3),
    )
    const wave = String.fromCharCode(
      view.getUint8(8),
      view.getUint8(9),
      view.getUint8(10),
      view.getUint8(11),
    )
    expect(riff).toBe('RIFF')
    expect(wave).toBe('WAVE')
    expect(view.getUint32(4, true)).toBe(36 + samples.length * 2)
    expect(view.getUint32(24, true)).toBe(16000)
    expect(view.getUint16(22, true)).toBe(1)
    expect(view.getUint16(34, true)).toBe(16)
  })

  it('clamps samples outside [-1, 1] to int16 extremes', async () => {
    const samples = new Float32Array([2, -2])
    const blob = pcmFloat32ToWavBlob(samples)
    const view = new DataView(await blob.arrayBuffer())
    expect(view.getInt16(44, true)).toBe(0x7fff)
    expect(view.getInt16(46, true)).toBe(-0x8000)
  })

  it('produces an empty data section for a zero-length sample array', async () => {
    const blob = pcmFloat32ToWavBlob(new Float32Array(0))
    expect(blob.size).toBe(44)
    const view = new DataView(await blob.arrayBuffer())
    expect(view.getUint32(40, true)).toBe(0)
  })
})
