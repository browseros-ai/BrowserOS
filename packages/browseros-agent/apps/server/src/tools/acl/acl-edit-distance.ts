export function editDistanceRatio(a: string, b: string): number {
  const maxLength = Math.max(a.length, b.length)
  if (maxLength === 0) return 1.0

  let previousRow = Array.from({ length: b.length + 1 }, (_, index) => index)
  let currentRow = new Array<number>(b.length + 1)

  for (let i = 1; i <= a.length; i++) {
    currentRow[0] = i

    for (let j = 1; j <= b.length; j++) {
      const substitutionCost = a[i - 1] === b[j - 1] ? 0 : 1
      currentRow[j] = Math.min(
        previousRow[j] + 1,
        currentRow[j - 1] + 1,
        previousRow[j - 1] + substitutionCost,
      )
    }

    ;[previousRow, currentRow] = [currentRow, previousRow]
  }

  const distance = previousRow[b.length]
  return 1.0 - distance / maxLength
}
