// Read inside an iframe two ways: a frame locator, and the frame from
// page.frames(). A srcdoc frame shares its parent's origin.
const html = '<title>Frames</title><p>Outside</p>'
  + '<iframe srcdoc="<p>Inside the frame</p><button>Frame action</button>"></iframe>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const frame = page.frameLocator('iframe');
return {
  button: await frame.getByRole('button').textContent(),
  text: await frame.locator('p').textContent(),
  frames: page.frames().length,
  fromFrames: await page.frames()[1].locator('p').textContent(),
};
