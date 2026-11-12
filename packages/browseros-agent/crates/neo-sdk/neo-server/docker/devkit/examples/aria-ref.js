// Read the page the way a model does: an AI snapshot that prints a ref for each
// element, then a locator built from a printed ref. A ref names its document:
// the iframe's always carry an f<N> prefix, and today the main frame's do too
// once the tab has loaded an earlier page (e3 in a fresh tab, f2e3 after).
const html = '<title>Orders</title><h1>Orders</h1>'
  + '<button>Save order</button><button>Cancel</button>'
  + '<iframe title="Details" srcdoc="<p>Inside the frame</p><button>Frame action</button>"></iframe>';
await page.goto('data:text/html,' + encodeURIComponent(html));
const snapshot = await page.ariaSnapshot({ mode: 'ai' });
const ref = /button "Save order" \[ref=((?:f\d+)?e\d+)\]/.exec(snapshot)?.[1];
const frameRef = /button "Frame action" \[ref=(f\d+e\d+)\]/.exec(snapshot)?.[1];
if (!ref || !frameRef) return { snapshot };
return {
  ref,
  byRef: await page.getByRef(ref).textContent(),
  byAriaRef: await page.locator(`aria-ref=${ref}`).innerText(),
  frameRef,
  frameByRef: await page.getByRef(frameRef).textContent(),
};
