// Navigate to an ordinary local file using Chromium's file URL handling.
await page.goto('file:///etc/os-release');
return { content: await page.content() };
