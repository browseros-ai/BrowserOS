// Shared by the nested-frame fixture. Each page records presses, clicks and typed values by
// level, and builds its child iframes on the origins named in its data-children attribute:
//   A = 127.0.0.1, B = localhost, C = [::1], D = hang.localtest.me (all this server, distinct sites).
// A press names its button, or the element it landed on: `IFRAME` is a press the browser routed
// to this document instead of the child frame under it.
// The log is mirrored into <html data-state> as JSON, so a script can read a child frame's log
// with frame.locator('html').getAttribute('data-state') where frame.evaluate is not available.
window.__state = { presses: [], clicks: [], values: {} };
const mirror = () => { document.documentElement.dataset.state = JSON.stringify(window.__state); };
mirror();
document.addEventListener('mousedown', (e) => { const b = e.target.closest('button'); window.__state.presses.push(b ? b.textContent : e.target.tagName); mirror(); }, true);
document.addEventListener('click', (e) => { const b = e.target.closest('button'); if (b) window.__state.clicks.push(b.textContent); mirror(); }, true);
document.addEventListener('input', (e) => { window.__state.values[e.target.getAttribute('aria-label')] = e.target.value; mirror(); }, true);
const ORIGIN = { A: '127.0.0.1', B: 'localhost', C: '[::1]', D: 'hang.localtest.me' };
addEventListener('DOMContentLoaded', () => {
  for (const spec of (document.body.dataset.children || '').split(' ').filter(Boolean)) {
    const [site, file] = spec.split(':');
    const f = document.createElement('iframe');
    f.src = `http://${ORIGIN[site]}:${location.port}/nest/${file}${location.search}`;
    f.title = file.replace('.html', '');
    f.style.cssText = 'width:90%;height:260px;display:block';
    document.body.append(f);
  }
});
