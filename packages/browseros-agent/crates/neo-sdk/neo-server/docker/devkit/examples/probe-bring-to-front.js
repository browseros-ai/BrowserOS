// Bringing a tab to the front. Refused on purpose: Neo keeps agent tabs in the background, so
// the owner's screen never jumps. Act on the page directly; the host's `followTask` or a
// takeover brings a tab forward for a person.
await page.goto('data:text/html,' + encodeURIComponent('<title>Front</title>'));
await page.bringToFront();
return 'in front';
