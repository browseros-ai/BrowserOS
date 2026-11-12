// A fixed wait. Works: page.waitForTimeout is a timer in the SDK, not a browser operation, so it
// shows in the run's time but not as an operation in the trace.
const started = Date.now();
await page.waitForTimeout(1500);
return { waitedMs: Date.now() - started };
