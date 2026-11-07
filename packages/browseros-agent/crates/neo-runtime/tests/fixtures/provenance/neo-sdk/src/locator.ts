/**
 * A fixture SDK file: a port of one Playwright class.
 *
 * Playwright: `server/frames.ts` `Frame`
 * Deviation: `server/frames.ts` `Frame.ariaSnapshot` Absent: the fixture serves no
 * snapshots.
 * Unexplained: why the fixture's locator keeps no timeout of its own.
 */

export class Locator {
  click(): never {
    throw new Error('Element is not visible')
  }
}
