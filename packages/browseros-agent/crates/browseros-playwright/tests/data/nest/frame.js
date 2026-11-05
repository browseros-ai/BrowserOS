// Shared by the nested-frame fixture. Each page records clicks and typed values by level, and
// builds its child iframes on the origins named in its data-children attribute:
//   A = 127.0.0.1, B = localhost, C = [::1], D = hang.localtest.me (all this server, distinct sites).
window.__state = { clicks: [], values: {} };
document.addEventListener('click', (e) => { const b = e.target.closest('button'); if (b) window.__state.clicks.push(b.textContent); }, true);
document.addEventListener('input', (e) => { window.__state.values[e.target.getAttribute('aria-label')] = e.target.value; }, true);
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
