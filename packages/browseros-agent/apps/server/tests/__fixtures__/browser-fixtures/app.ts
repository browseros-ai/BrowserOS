import http, {
  type IncomingMessage,
  type Server,
  type ServerResponse,
} from 'node:http'

export const fixtureRoutes = {
  index: '/',
  clicks: '/clicks',
  frameClick: '/frame-click',
  snapshotDiff: '/snapshot-diff',
  formControls: '/form-controls',
  upload: '/upload',
  uploadPopupLauncher: '/upload-popup',
  uploadPopupWindow: '/upload-popup-window',
  spa: '/spa',
} as const

export const fixtureRouteList = [
  {
    path: fixtureRoutes.clicks,
    title: 'Click targets',
    description: 'Normal, iframe, out-of-viewport, and zero-geometry buttons.',
  },
  {
    path: fixtureRoutes.snapshotDiff,
    title: 'Snapshot diff',
    description: 'One click removes five controls and adds ten new controls.',
  },
  {
    path: fixtureRoutes.formControls,
    title: 'Form controls',
    description: 'Checkbox and select inputs with visible state output.',
  },
  {
    path: fixtureRoutes.upload,
    title: 'File upload',
    description: 'Single file input with uploaded file metadata.',
  },
  {
    path: fixtureRoutes.uploadPopupLauncher,
    title: 'Popup upload',
    description: 'Launcher that opens a popup-style upload page.',
  },
  {
    path: fixtureRoutes.spa,
    title: 'SPA navigation',
    description: 'History API route changes without full page reloads.',
  },
] as const

export interface BrowserFixtureServer {
  baseUrl: string
  url(path: string): string
  stop(): Promise<void>
}

export interface StartOptions {
  port?: number
}

function randomPort(): number {
  const min = 10101
  const max = 20202
  return Math.floor(Math.random() * (max - min + 1) + min)
}

function page(title: string, body: string): string {
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${title}</title>
  <style>
    :root {
      color-scheme: light;
      font-family: Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      color: #1d2433;
      background: #f7f9fc;
    }
    body {
      margin: 0;
      min-height: 100vh;
    }
    header {
      background: #fff;
      border-bottom: 1px solid #d9e0ea;
      padding: 18px 24px;
      position: sticky;
      top: 0;
      z-index: 1;
    }
    main {
      max-width: 980px;
      padding: 24px;
    }
    h1 {
      margin: 0;
      font-size: 24px;
      line-height: 1.2;
    }
    h2 {
      font-size: 18px;
      margin: 28px 0 12px;
    }
    a {
      color: #0f766e;
    }
    button, select, input[type="file"] {
      font: inherit;
    }
    button {
      border: 1px solid #97a6ba;
      border-radius: 6px;
      background: #fff;
      color: #1d2433;
      min-height: 36px;
      padding: 7px 12px;
      cursor: pointer;
    }
    button:hover {
      border-color: #0f766e;
    }
    .grid {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
      gap: 12px;
    }
    .panel {
      background: #fff;
      border: 1px solid #d9e0ea;
      border-radius: 8px;
      padding: 16px;
    }
    .status {
      background: #eef6f4;
      border: 1px solid #bddbd5;
      border-radius: 6px;
      min-height: 24px;
      padding: 8px 10px;
      margin-top: 10px;
      font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      font-size: 13px;
      white-space: pre-wrap;
    }
    iframe {
      width: 100%;
      height: 180px;
      border: 1px solid #c4cedb;
      border-radius: 6px;
      background: #fff;
    }
    label {
      display: block;
      margin: 10px 0 6px;
    }
    .spacer {
      height: 1250px;
      border-left: 2px dashed #c4cedb;
      margin: 18px 0;
      padding-left: 14px;
      color: #536176;
    }
    .zero-geometry {
      position: absolute;
      width: 0;
      height: 0;
      padding: 0;
      border: 0;
      overflow: hidden;
      clip-path: inset(50%);
    }
    nav.route-list {
      display: grid;
      gap: 10px;
      margin-top: 18px;
    }
    nav.route-list a {
      display: block;
      background: #fff;
      border: 1px solid #d9e0ea;
      border-radius: 8px;
      padding: 14px 16px;
      color: inherit;
      text-decoration: none;
    }
    nav.route-list strong {
      display: block;
      margin-bottom: 4px;
    }
  </style>
</head>
<body>
  <header><h1>${title}</h1></header>
  <main>${body}</main>
</body>
</html>`
}

function renderIndex(baseUrl: string): string {
  const links = fixtureRouteList
    .map(
      (route) => `<a href="${route.path}">
        <strong>${route.title}</strong>
        <span>${route.description}</span>
      </a>`,
    )
    .join('\n')

  return page(
    'BrowserOS tool fixtures',
    `<p>Local pages for exercising browser tool migrations. Base URL: <code>${baseUrl}</code></p>
    <nav class="route-list">${links}</nav>`,
  )
}

function renderClicks(): string {
  return page(
    'Click target fixtures',
    `<section class="grid">
      <div class="panel">
        <h2>Normal click</h2>
        <button id="normal-click">Normal click target</button>
        <div id="normal-status" class="status">normal:0</div>
      </div>
      <div class="panel">
        <h2>Iframe click</h2>
        <iframe title="Frame click fixture" src="${fixtureRoutes.frameClick}"></iframe>
        <div id="frame-status" class="status">frame:0</div>
      </div>
      <div class="panel">
        <h2>Zero geometry</h2>
        <button id="zero-geometry-click" class="zero-geometry" aria-label="Zero geometry click target">Zero geometry click target</button>
        <div id="zero-status" class="status">zero:0</div>
      </div>
    </section>
    <div class="spacer">Scroll target is below this divider.</div>
    <section class="panel">
      <h2>Out of viewport</h2>
      <button id="deep-click">Out of viewport click target</button>
      <div id="deep-status" class="status">deep:0</div>
    </section>
    <script>
      window.fixtureClicks = { normal: 0, frame: 0, zero: 0, deep: 0 };
      function status(id, key) {
        document.getElementById(id).textContent = key + ':' + window.fixtureClicks[key];
      }
      document.getElementById('normal-click').addEventListener('click', () => {
        window.fixtureClicks.normal++;
        status('normal-status', 'normal');
      });
      document.getElementById('zero-geometry-click').addEventListener('click', () => {
        window.fixtureClicks.zero++;
        status('zero-status', 'zero');
      });
      document.getElementById('deep-click').addEventListener('click', () => {
        window.fixtureClicks.deep++;
        status('deep-status', 'deep');
      });
      window.addEventListener('message', (event) => {
        if (event.data && event.data.type === 'frame-click') {
          window.fixtureClicks.frame = event.data.count;
          status('frame-status', 'frame');
        }
      });
    </script>`,
  )
}

function renderFrameClick(): string {
  return page(
    'Frame click fixture',
    `<section class="panel">
      <button id="frame-click">Frame click target</button>
      <div id="frame-local-status" class="status">frame-local:0</div>
    </section>
    <script>
      window.fixtureFrameClicks = 0;
      document.getElementById('frame-click').addEventListener('click', () => {
        window.fixtureFrameClicks++;
        document.getElementById('frame-local-status').textContent = 'frame-local:' + window.fixtureFrameClicks;
        window.parent.postMessage({ type: 'frame-click', count: window.fixtureFrameClicks }, '*');
      });
    </script>`,
  )
}

function renderSnapshotDiff(): string {
  const oldButtons = Array.from(
    { length: 5 },
    (_, index) =>
      `<button class="old-action" id="old-action-${index + 1}">Old action ${index + 1}</button>`,
  ).join('\n')

  return page(
    'Snapshot diff fixture',
    `<section class="panel">
      <button id="mutate-snapshot">Reveal ten new actions</button>
      <div id="diff-status" class="status">before mutation</div>
    </section>
    <section id="old-actions" class="grid" aria-label="Old actions">${oldButtons}</section>
    <section id="new-actions" class="grid" aria-label="New actions"></section>
    <script>
      document.getElementById('mutate-snapshot').addEventListener('click', () => {
        document.getElementById('old-actions').innerHTML = '';
        document.getElementById('new-actions').innerHTML = Array.from({ length: 10 }, (_, index) => {
          const n = index + 1;
          return '<button class="new-action" id="new-action-' + n + '">New action ' + n + '</button>';
        }).join('');
        document.getElementById('diff-status').textContent = 'after mutation: added 10 removed 5';
      });
    </script>`,
  )
}

function renderFormControls(): string {
  return page(
    'Form control fixtures',
    `<section class="panel">
      <label><input id="updates-checkbox" type="checkbox"> Receive updates</label>
      <div id="checkbox-status" class="status">checkbox:false</div>
    </section>
    <section class="panel">
      <label for="plan-select">Plan select</label>
      <select id="plan-select">
        <option value="starter">Starter</option>
        <option value="team">Team</option>
        <option value="enterprise">Enterprise</option>
      </select>
      <div id="select-status" class="status">select:starter</div>
    </section>
    <script>
      const checkbox = document.getElementById('updates-checkbox');
      const select = document.getElementById('plan-select');
      window.fixtureForm = { checked: false, selected: 'starter' };
      checkbox.addEventListener('change', () => {
        window.fixtureForm.checked = checkbox.checked;
        document.getElementById('checkbox-status').textContent = 'checkbox:' + checkbox.checked;
      });
      select.addEventListener('change', () => {
        window.fixtureForm.selected = select.value;
        document.getElementById('select-status').textContent = 'select:' + select.value;
      });
    </script>`,
  )
}

function renderUpload(title = 'File upload fixture'): string {
  return page(
    title,
    `<section class="panel">
      <label for="fixture-upload">Upload file</label>
      <input id="fixture-upload" type="file">
      <div id="upload-status" class="status">upload:none</div>
    </section>
    <script>
      window.fixtureUpload = { names: [] };
      document.getElementById('fixture-upload').addEventListener('change', (event) => {
        window.fixtureUpload.names = Array.from(event.target.files || []).map((file) => file.name);
        document.getElementById('upload-status').textContent = 'upload:' + window.fixtureUpload.names.join(',');
      });
    </script>`,
  )
}

function renderUploadPopupLauncher(): string {
  return page(
    'Popup upload launcher',
    `<section class="panel">
      <button id="open-upload-popup">Open upload popup</button>
      <div id="popup-status" class="status">popup:closed</div>
    </section>
    <script>
      document.getElementById('open-upload-popup').addEventListener('click', () => {
        const popup = window.open('${fixtureRoutes.uploadPopupWindow}', 'browseros-fixture-upload', 'width=520,height=420');
        document.getElementById('popup-status').textContent = popup ? 'popup:opened' : 'popup:blocked';
      });
    </script>`,
  )
}

function renderSpa(pathname: string): string {
  const initialView = pathname.endsWith('/settings')
    ? 'settings'
    : pathname.endsWith('/details')
      ? 'details'
      : 'home'

  return page(
    'SPA navigation fixture',
    `<section class="panel">
      <button data-route="home">SPA Home</button>
      <button data-route="details">SPA Details</button>
      <button data-route="settings">SPA Settings</button>
      <div id="spa-view" class="status"></div>
    </section>
    <script>
      const initialView = ${JSON.stringify(initialView)};
      const views = {
        home: 'Home view loaded',
        details: 'Details view loaded',
        settings: 'Settings view loaded'
      };
      function setView(view, push) {
        window.fixtureSpaView = view;
        document.getElementById('spa-view').textContent = views[view];
        if (push) {
          const path = view === 'home' ? '${fixtureRoutes.spa}' : '${fixtureRoutes.spa}/' + view;
          history.pushState({ view }, '', path);
        }
      }
      document.querySelectorAll('[data-route]').forEach((button) => {
        button.addEventListener('click', () => setView(button.dataset.route, true));
      });
      window.addEventListener('popstate', (event) => setView((event.state && event.state.view) || 'home', false));
      history.replaceState({ view: initialView }, '', location.pathname);
      setView(initialView, false);
    </script>`,
  )
}

function renderPath(pathname: string, baseUrl: string): string | undefined {
  if (pathname === fixtureRoutes.index) return renderIndex(baseUrl)
  if (pathname === fixtureRoutes.clicks) return renderClicks()
  if (pathname === fixtureRoutes.frameClick) return renderFrameClick()
  if (pathname === fixtureRoutes.snapshotDiff) return renderSnapshotDiff()
  if (pathname === fixtureRoutes.formControls) return renderFormControls()
  if (pathname === fixtureRoutes.upload) return renderUpload()
  if (pathname === fixtureRoutes.uploadPopupLauncher) {
    return renderUploadPopupLauncher()
  }
  if (pathname === fixtureRoutes.uploadPopupWindow) {
    return renderUpload('Upload popup fixture')
  }
  if (pathname === fixtureRoutes.spa || pathname.startsWith('/spa/')) {
    return renderSpa(pathname)
  }
  return undefined
}

/** Starts the human-viewable browser fixture app used by migration tests. */
export async function startBrowserFixtureServer(
  opts: StartOptions = {},
): Promise<BrowserFixtureServer> {
  const port = opts.port ?? randomPort()
  const baseUrl = `http://localhost:${port}`
  const server: Server = http.createServer(
    (req: IncomingMessage, res: ServerResponse) => {
      const url = new URL(req.url ?? '/', baseUrl)
      const html = renderPath(url.pathname, baseUrl)
      if (!html) {
        res.writeHead(404, { 'Content-Type': 'text/plain; charset=utf-8' })
        res.end('Not found')
        return
      }
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' })
      res.end(html)
    },
  )

  await new Promise<void>((resolve) => server.listen(port, resolve))

  return {
    baseUrl,
    url(path: string): string {
      return `${baseUrl}${path}`
    },
    stop(): Promise<void> {
      return new Promise((resolve, reject) => {
        server.close((err) => {
          if (err) reject(err)
          else resolve()
        })
      })
    },
  }
}
