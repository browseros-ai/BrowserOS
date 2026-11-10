/**
 * Fixture for neo-runtime's provenance tests, shaped like Playwright's server/frames.ts.
 */

export class Frame {
  async expect(progress: Progress, selector: string | undefined, options: FrameExpectParams): Promise<ExpectResult> {
    let received: ExpectReceived = {};
    if (options.isNot) {
      const ariaSnapshot = await progress.race(injected.evaluate(injected => {
        return injected.ariaSnapshotForExpectFailure(injected.document.body, { mode: 'default' });
      }));
      if (ariaSnapshot)
        received = { ...received, ariaSnapshot };
    }
    return { received };
  }

  async click(progress: Progress, selector: string, options: ClickOptions) {
    const waitTime = [0, 20, 100, 100, 500];
    for (let retry = 0; ; ++retry) {
      if (retry)
        progress.log(`retrying click action${options.trial ? ' (trial run)' : ''}`);
      else
        progress.log(`attempting click action${options.trial ? ' (trial run)' : ''}`);
      const timeout = waitTime[Math.min(retry, waitTime.length - 1)];
      if (timeout)
        progress.log(`  waiting ${timeout}ms`);
      if (!await this._isVisible(selector))
        throw new Error('Element is not visible');
      return;
    }
  }

  async ariaSnapshot(progress: Progress, options: { mode?: 'ai' | 'default' } = {}): Promise<{ snapshot: string }> {
    const { snapshot } = await this.ariaSnapshotJSON(progress, options);
    return { snapshot: renderAriaSnapshotAsYaml(snapshot) };
  }
}

export function quadMiddlePoint(quad: Quad): Point {
  const result = { x: 0, y: 0 };
  for (const point of quad) {
    result.x += point.x / 4;
    result.y += point.y / 4;
  }
  return result;
}
