/** Decode only after both sides returned real images. Dimensions are exact; channel noise up
 * to 2/255 is tolerated on at most 0.1% of pixels. No resizing or page-value normalization. */

import { writeFileSync } from 'node:fs'
import jpeg from 'jpeg-js'
import { PNG } from 'pngjs'
export function compareImages(native, neo, directory) {
  const results = []
  if (native.length !== neo.length || native.length === 0)
    return { ok: false, reason: 'image count differs or empty' }
  for (let i = 0; i < native.length; i++) {
    const p = native[i],
      n = neo[i]
    const decode = (image) =>
      image.format === 'jpeg'
        ? jpeg.decode(Buffer.from(image.base64, 'base64'))
        : PNG.sync.read(Buffer.from(image.base64, 'base64'))
    if (p.name !== n.name || p.format !== n.format)
      return { ok: false, reason: 'image identities differ' }
    const a = decode(p),
      b = decode(n)
    writeFileSync(
      `${directory}/${i}-native.${p.format}`,
      Buffer.from(p.base64, 'base64'),
    )
    writeFileSync(
      `${directory}/${i}-neo.${n.format}`,
      Buffer.from(n.base64, 'base64'),
    )
    if (a.width !== b.width || a.height !== b.height)
      return {
        ok: false,
        reason: 'dimensions differ',
        native: [a.width, a.height],
        neo: [b.width, b.height],
      }
    let different = 0
    for (let px = 0; px < a.width * a.height; px++)
      if (
        [0, 1, 2, 3].some(
          (channel) =>
            Math.abs(a.data[px * 4 + channel] - b.data[px * 4 + channel]) > 2,
        )
      )
        different++
    results.push({
      name: p.name,
      width: a.width,
      height: a.height,
      differentPixels: different,
      fraction: different / (a.width * a.height),
    })
  }
  return { ok: results.every((r) => r.fraction <= 0.001), results }
}
