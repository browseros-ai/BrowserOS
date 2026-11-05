// Close the task's tab. A takeover leaves its tab private until it closes, and
// view displays stay paused while a private tab is in front.
await page.close();
return { closed: true };
