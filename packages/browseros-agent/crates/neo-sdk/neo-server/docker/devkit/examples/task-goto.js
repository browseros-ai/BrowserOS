// A write on the task's tab: refused while the host holds a lock or a takeover
// on that tab (host / owner_has_control, delivery not_sent), done otherwise.
await page.goto('https://example.com/?from=task-goto');
return { url: page.url() };
