#!/usr/bin/env python3
"""accept.sh's probes, run inside the containers under test with the image's own python3.

accept.sh copies this file next to the Neo image's smoke.py (whose streamable-HTTP MCP client
it reuses) and runs one command per call, as whichever uid the check needs. Each command prints
one JSON object on stdout; accept.sh decides PASS or FAIL from it.

  run <mcp-url> <task-id> <request-id>   a `run` that drives a real page, with the host's _meta
  downloads <mcp-url> <socket> <token-file>
                                         a click that downloads, `path()` and `saveAs()`, the
                                         bytes over the host socket, then the same path uploaded
  pdf-downloads <mcp-url> <socket> <token-file>
                                         PDF viewer and download cases, with a ZIP control
  blobs <mcp-url> <socket> <token-file>  a full-page PNG and a PDF too large to return inline,
                                         read back as blobs; large text still refused
  describe <mcp-url>                     the `run` tool's description, which lists neo.* methods
  host <socket> <token-file|-> <path>    GET a host API path over the Unix socket
  listeners                              the container's listening TCP sockets, from /proc
  probe <host> <port> http|https         GET /system/ready from another container

Standard library only.
"""

import hashlib
import http.server
import io
import json
import os
import socket
import ssl
import sys
import threading
import urllib.parse
import zipfile

RUN_META = "com.browseros.neo/run"
TITLE = "neo-sdk-accept"
# A data: page in the task's own tab, filled and read back through Playwright locators: the
# shape of call a migrated zuzu mirrord sends. The image under test has no network.
CODE = f"""\
await page.goto('data:text/html,<title>{TITLE}</title><h1 id=h>hello</h1><input aria-label=Name>');
await page.getByLabel('Name').fill('zuzu');
return {{ title: await page.title(), heading: await page.locator('#h').textContent(),
         value: await page.getByLabel('Name').inputValue() }};
"""


def connect(url: str):
    # Imported here so `probe` runs from stdin in a container that has only this file.
    from smoke import PROTOCOL, McpClient

    client = McpClient(url)
    client.call(
        "initialize",
        {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "neo-sdk-accept", "version": "1"},
        },
    )
    client.notify("notifications/initialized")
    return client


def describe(url: str) -> dict:
    tools = connect(url).tools()
    return {"description": next(t.get("description", "") for t in tools if t["name"] == "run")}


def call_run(client, code: str, meta: dict) -> dict:
    result = client.call(
        "tools/call",
        {
            "name": "run",
            "arguments": {"code": code, "timeout": 60_000},
            "_meta": {RUN_META: meta},
        },
    )
    text = "\n".join(p.get("text", "") for p in result.get("content", []) if p.get("type") == "text")
    return {
        "isError": bool(result.get("isError")),
        "meta": result.get("_meta", {}).get(RUN_META, {}),
        "value": result.get("structuredContent"),
        "text": text[:400],
    }


def run(url: str, task_id: str, request_id: str) -> dict:
    client = connect(url)
    tools = [tool["name"] for tool in client.tools()]
    meta = {"requestId": request_id, "taskId": task_id, "hostData": {"check": "accept"}}
    return {"tools": tools, **call_run(client, CODE, meta)}


def host_raw(socket_path: str, token_file: str, path: str, method: str = "GET", body: bytes = b""):
    """`method` `path` over the host socket, with a JSON `body` when one is given: (status,
    lower-cased headers, body), or the connect error."""
    token = "" if token_file == "-" else open(token_file).read().strip()
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.settimeout(20)
    try:
        connection.connect(socket_path)
    except OSError as error:
        return {"connect": error.__class__.__name__, "errno": error.errno}
    auth = f"Authorization: Bearer {token}\r\n" if token else ""
    content = f"Content-Type: application/json\r\nContent-Length: {len(body)}\r\n" if body else ""
    connection.sendall(
        f"{method} {path} HTTP/1.1\r\nHost: neo\r\n{auth}{content}Connection: close\r\n\r\n".encode() + body
    )
    reply = b""
    while chunk := connection.recv(65536):
        reply += chunk
    connection.close()
    head, _, body = reply.partition(b"\r\n\r\n")
    lines = head.decode(errors="replace").split("\r\n")
    status = int(lines[0].split(" ", 2)[1])
    headers = {}
    for line in lines[1:]:
        name, _, value = line.partition(":")
        headers[name.strip().lower()] = value.strip()
    if headers.get("transfer-encoding", "").lower() == "chunked":
        body = dechunk(body)
    return status, headers, body


def host(socket_path: str, token_file: str, path: str, method: str = "GET", body: str = "") -> dict:
    raw = host_raw(socket_path, token_file, path, method, body.encode())
    if isinstance(raw, dict):
        return raw
    status, _, body = raw
    try:
        parsed = json.loads(body)
    except ValueError:
        parsed = body.decode(errors="replace")[:400]
    return {"connect": "ok", "status": status, "body": parsed}


# What `downloads` serves as a download: a CSV a bank site would, with a name the browser keeps.
STATEMENT_NAME = "statement-2026-10.csv"
STATEMENT = b"".join(b"2026-10-%02d,coffee,%d.50\r\n" % (day % 28 + 1, day) for day in range(4000))


class FilesPage(http.server.BaseHTTPRequestHandler):
    """A page with a download link and a file input, on 127.0.0.1: the check's network has no
    route out. 127.0.0.1 is a secure context, so the page can hash what it was given."""

    def do_GET(self):
        if self.path == "/statement":
            body, kind = STATEMENT, "text/csv"
            disposition = f'attachment; filename="{STATEMENT_NAME}"'
        else:
            body, kind, disposition = (
                b"<title>files</title><a id=get href=/statement>Statement</a><input type=file id=f>",
                "text/html",
                None,
            )
        self.send_response(200)
        self.send_header("Content-Type", kind)
        if disposition:
            self.send_header("Content-Disposition", disposition)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_args):
        pass


DOWNLOAD_CODE = """\
await page.goto('ORIGIN/');
const [download] = await Promise.all([page.waitForEvent('download'), page.locator('#get').click()]);
const path = await download.path();
await download.saveAs('/tmp/neo-accept-saved.csv');
return { path, name: download.suggestedFilename(), failure: await download.failure() };
"""

UPLOAD_CODE = """\
await page.goto('ORIGIN/');
await page.locator('#f').setInputFiles(args.path);
return await page.locator('#f').evaluate(async (input) => {
  const file = input.files[0];
  const digest = await crypto.subtle.digest('SHA-256', await file.arrayBuffer());
  return { name: file.name, size: file.size,
           sha256: [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('') };
});
"""


def downloads(url: str, socket_path: str, token_file: str) -> dict:
    """zuzu's neo_download then neo_upload, and `import` reading the bytes on the host socket."""
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), FilesPage)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f"http://127.0.0.1:{server.server_address[1]}"
    want = hashlib.sha256(STATEMENT).hexdigest()
    client = connect(url)
    problems = []
    got = call_run(client, DOWNLOAD_CODE.replace("ORIGIN", origin), {"taskId": "accept-downloads"})
    value = (got["value"] or {}).get("value") or {}
    run_id = got["meta"].get("id", "")
    path = value.get("path") or ""
    if got["meta"].get("status") != "ok" or not path.startswith("/data/downloads/"):
        problems.append(f"download run: {got['meta'].get('status')} {got['text'][:200]}")
    if value.get("name") != STATEMENT_NAME or value.get("failure") is not None:
        problems.append(f"download facts: {value}")
    record = host(socket_path, token_file, f"/v1/runs/{run_id}?wait=10")
    listed = ((record.get("body") or {}).get("downloads") or [{}])[0]
    if (listed.get("state"), listed.get("size"), listed.get("path")) != ("completed", len(STATEMENT), path):
        problems.append(f"record: {listed}")
    raw = host_raw(socket_path, token_file, f"/v1/runs/{run_id}/downloads/0")
    status, headers, body = raw if isinstance(raw, tuple) else (raw, {}, b"")
    if status != 200 or hashlib.sha256(body).hexdigest() != want:
        problems.append(f"bytes: {status}, {len(body)} bytes")
    if STATEMENT_NAME not in headers.get("content-disposition", ""):
        problems.append(f"content-disposition: {headers.get('content-disposition')}")
    try:
        saved = os.stat("/tmp/neo-accept-saved.csv").st_size
    except OSError as error:
        saved = str(error)
    if saved != len(STATEMENT):
        problems.append(f"saveAs copy: {saved}")
    upload_code = f"const args = {json.dumps({'path': path})};\n" + UPLOAD_CODE.replace("ORIGIN", origin)
    uploaded = call_run(client, upload_code, {"taskId": "accept-downloads"})
    sent = (uploaded["value"] or {}).get("value") or {}
    if (sent.get("name"), sent.get("sha256")) != (os.path.basename(path), want):
        problems.append(f"upload: {uploaded['meta'].get('status')} {sent or uploaded['text'][:200]}")
    server.shutdown()
    return {
        "ok": not problems,
        "run": run_id,
        "problems": problems,
        "detail": f"run {run_id}: {path}, {len(STATEMENT)} bytes read back over the socket and uploaded again",
    }


def pdf_fixture() -> bytes:
    """A deterministic one-page PDF with real cross-reference offsets, so the viewer can open it."""
    objects = [b"<< /Type /Catalog /Pages 2 0 R >>", b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
               b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>",
               b"<< /Length 0 >>\nstream\nendstream"]
    body, offsets = b"%PDF-1.4\n", [0]
    for index, obj in enumerate(objects, 1):
        offsets.append(len(body))
        body += f"{index} 0 obj\n".encode() + obj + b"\nendobj\n"
    xref = len(body)
    body += b"xref\n0 5\n0000000000 65535 f \n"
    body += b"".join(f"{offset:010} 00000 n \n".encode() for offset in offsets[1:])
    return body + f"trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()


PDF = pdf_fixture()
ZIP_BUFFER = io.BytesIO()
with zipfile.ZipFile(ZIP_BUFFER, "w") as archive:
    archive.writestr(zipfile.ZipInfo("control.txt", (2026, 1, 1, 0, 0, 0)), b"complete ZIP bytes\n")
# Keep the host's original PDF-prefix payload separate from the four valid PDF scenarios.
PDF_CASES = (
    ("report.pdf", b"%PDF-1.4\n" + b"complete PDF bytes\n" * 100000, "host"),
    ("pdf-attachment.pdf", PDF, "attachment"),
    ("pdf-inline.pdf", PDF, "inline"),
    ("pdf-link.pdf", PDF, "download"),
    ("pdf-blob.pdf", PDF, "blob"),
    ("zip-attachment.zip", ZIP_BUFFER.getvalue(), "attachment"),
)


class PdfFilesPage(FilesPage):
    """The six local fixtures share an origin, including the download-attribute and Blob cases."""

    def do_GET(self):
        if self.path == "/":
            body = b'<!doctype html><title>File proof</title><h1>File proof</h1><a href="/file" download>download</a>'
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        fixture = PDF_CASES[0] if self.path == "/file" else next(
            (case for case in PDF_CASES if self.path == "/" + case[0]), None)
        if fixture is None:
            return super().do_GET()
        name, body, mode = fixture
        self.send_response(200)
        self.send_header("Content-Type", "application/zip" if name.endswith(".zip") else "application/pdf")
        self.send_header("Content-Disposition", f'{"attachment" if mode in ("host", "attachment") else "inline"}; filename="{name}"')
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


# Plain Playwright calls and explicit args let the same body run against upstream Chromium.
PDF_DOWNLOAD_CODE = """\
const p = await context.newPage();
const facts = {};
const observe = async (promise) => {
  let timer;
  try {
    return await Promise.race([
      promise.then(value => ({ value: value === undefined ? null : value }), error => ({ error: String(error) })),
      new Promise(resolve => { timer = setTimeout(() => resolve({ error: 'observation timeout' }), 8000); })
    ]);
  } finally { clearTimeout(timer); }
};
try {
  await p.goto(args.origin + '/', { timeout: 12000 });
  // Register before dispatch. A navigation that becomes a download may reject independently.
  const waiting = p.waitForEvent('download', { timeout: 15000 })
    .then(download => ({ download }), error => ({ error: String(error) }));
  try {
    if (args.mode === 'host') {
      await p.getByRole('link', { name: 'download', exact: true }).click({ timeout: 12000 });
    } else if (args.mode === 'download' || args.mode === 'blob') {
      await p.evaluate(async ({ url, mode, name }) => {
        const a = document.createElement('a');
        a.download = name;
        a.href = mode === 'blob'
          ? URL.createObjectURL(new Blob([await (await fetch(url)).arrayBuffer()], { type: 'application/pdf' }))
          : url;
        document.body.append(a);
        a.click();
      }, args);
    } else {
      await p.goto(args.url, { waitUntil: 'load', timeout: 12000 });
    }
  } catch (error) { facts.triggerError = String(error); }
  const event = args.mode === 'inline'
    ? await Promise.race([waiting, new Promise(resolve => setTimeout(() => resolve({ error: 'no event after load' }), 2000))])
    : await waiting;
  facts.event = event.download ? { name: event.download.suggestedFilename(), url: event.download.url() } : event;
  if (args.mode === 'inline') {
    // Chromium's PDF embed is hidden from document queries. Its generated document
    // references the viewer's extension stylesheet even when no download exists.
    facts.viewer = await p.evaluate(() => ({ contentType: document.contentType,
      stylesheet: [...document.querySelectorAll('link[rel="stylesheet"]')].some(link =>
        link.href.startsWith('chrome-extension://') && link.href.endsWith('/pdf_embedder.css')) }));
  } else if (event.download) {
    // These methods have no timeout option. Observe each independently so a stuck artifact
    // still returns evidence. Ending the run cancels operations left behind by the races.
    [facts.path, facts.saveAs, facts.failure] = await Promise.all([
      observe(event.download.path()), observe(event.download.saveAs(args.saved)), observe(event.download.failure())
    ]);
  }
} catch (error) { facts.error = String(error); }
finally { facts.close = await observe(p.close()); }
return facts;
"""


def pdf_downloads(url: str, socket_path: str, token_file: str) -> dict:
    """Verify browser files and host-import bytes. The collector needs root to read neo's /data."""
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), PdfFilesPage)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f"http://127.0.0.1:{server.server_address[1]}"
    problems, cases = [], []
    try:
        client = connect(url)
        for name, body, mode in PDF_CASES:
            saved = f"/tmp/neo-accept-{os.getpid()}-{name}"
            args = {"origin": origin, "url": origin + "/" + name, "mode": mode, "name": name, "saved": saved}
            before = set(os.listdir("/data/downloads"))
            evidence = {"case": name}
            cases.append(evidence)
            try:
                got = call_run(client, f"const args = {json.dumps(args)};\n" + PDF_DOWNLOAD_CODE,
                               {"taskId": "accept-pdf-downloads"})
                value = (got["value"] or {}).get("value") or {}
                run_id = got["meta"].get("id", "")
                record = host(socket_path, token_file, f"/v1/runs/{run_id}?wait=10")
                listed = (record.get("body") or {}).get("downloads") or []
                evidence.update(run=run_id, result=got, downloads=listed)
                assert got["meta"].get("status") == "ok" and not value.get("error"), str(got)
                assert record.get("status") == 200, str(record)
                if mode == "inline":
                    assert not value.get("triggerError"), str(value)
                    assert value.get("event") == {"error": "no event after load"} and not listed, str(value)
                    assert value.get("viewer") == {"contentType": "application/pdf", "stylesheet": True}, str(value)
                else:
                    path = (value.get("path") or {}).get("value") or ""
                    assert path.startswith("/data/downloads/"), str(value)
                    assert value.get("failure") == {"value": None} and "value" in value.get("saveAs", {}), str(value)
                    assert value.get("event", {}).get("name") == name, str(value)
                    want = hashlib.sha256(body).hexdigest()
                    evidence["sha256"] = {"expected": want}
                    for label, filename in (("original", path), ("saved", saved)):
                        with open(filename, "rb") as file:
                            evidence["sha256"][label] = hashlib.sha256(file.read()).hexdigest()
                        assert evidence["sha256"][label] == want, filename
                    assert len(listed) == 1 and (listed[0].get("state"), listed[0].get("size"), listed[0].get("path")) == ("completed", len(body), path), str(listed)
                    raw = host_raw(socket_path, token_file, f"/v1/runs/{run_id}/downloads/0")
                    assert isinstance(raw, tuple) and raw[0] == 200 and hashlib.sha256(raw[2]).hexdigest() == want, str(raw)[:200]
                    evidence["sha256"]["host"] = hashlib.sha256(raw[2]).hexdigest()
                assert not value.get("close", {}).get("error"), str(value)
            except Exception as error:
                problems.append(f"{name}: {error}; files={sorted(set(os.listdir('/data/downloads')) - before)}")
            finally:
                evidence["files"] = sorted(set(os.listdir("/data/downloads")) - before)
    finally:
        server.shutdown()
        server.server_close()
    return {"ok": not problems, "problems": problems, "cases": cases,
            "detail": "six PDF/ZIP cases: viewer retained; original, saveAs and host bytes match SHA-256"}


def dechunk(body: bytes) -> bytes:
    out = b""
    while body:
        size, _, rest = body.partition(b"\r\n")
        length = int(size, 16)
        if length == 0:
            break
        out += rest[:length]
        body = rest[length + 2 :]
    return out


def listeners() -> dict:
    """Listening sockets as "address:port", from /proc (the image has no ss)."""
    found = set()
    for table, width in (("/proc/net/tcp", 8), ("/proc/net/tcp6", 32)):
        try:
            rows = open(table).read().splitlines()[1:]
        except OSError:
            continue
        for row in rows:
            fields = row.split()
            if fields[3] != "0A":  # TCP_LISTEN
                continue
            address, port = fields[1].split(":")
            found.add(f"{decode_address(address, width)}:{int(port, 16)}")
    return {"listening": sorted(found)}


def decode_address(hex_address: str, width: int) -> str:
    raw = bytes.fromhex(hex_address)
    if width == 8:
        return socket.inet_ntop(socket.AF_INET, raw[::-1])
    words = b"".join(raw[i : i + 4][::-1] for i in range(0, 16, 4))
    return socket.inet_ntop(socket.AF_INET6, words)


def probe(host_name: str, port: int, scheme: str) -> dict:
    """What a peer outside the container's loopback gets from the browser's MCP proxy."""
    try:
        raw = socket.create_connection((host_name, port), timeout=10)
    except TimeoutError:
        return {"tcp": "timeout"}
    except OSError as error:
        return {"tcp": "refused", "error": str(error)}
    try:
        if scheme == "https":
            context = ssl.create_default_context()
            context.check_hostname = False
            context.verify_mode = ssl.CERT_NONE
            raw = context.wrap_socket(raw, server_hostname=host_name)
        raw.sendall(f"GET /system/ready HTTP/1.1\r\nHost: {host_name}\r\nConnection: close\r\n\r\n".encode())
        reply = raw.recv(4096)
    except TimeoutError:
        # Neither an answer nor a close: a firewall drop, or a hung listener.
        return {"tcp": "accepted", "http": "timeout"}
    except (OSError, ssl.SSLError) as error:
        return {"tcp": "accepted", "http": "closed", "error": error.__class__.__name__}
    finally:
        raw.close()
    if not reply:
        return {"tcp": "accepted", "http": "closed"}
    return {"tcp": "accepted", "http": reply.split(b"\r\n", 1)[0].decode(errors="replace")}


# A page whose full-page PNG cannot compress below 1.5 MB: 400 coloured rows and a canvas of
# random pixels, so the screenshot and its PDF are both over the 256 KiB inline limit.
BLOBS_CODE = """\
const rows = Array.from({ length: 400 }, (_, i) =>
  `<tr style="background:hsl(${(i * 47) % 360} 70% 60%)"><td>${i}</td><td>row ${i} of the statement</td></tr>`).join('');
await page.setContent(`<title>rows</title><canvas id=c width=1000 height=600></canvas><table>${rows}</table>`);
await page.evaluate(() => {
  const canvas = document.getElementById('c');
  const context = canvas.getContext('2d');
  const image = context.createImageData(canvas.width, canvas.height);
  for (let i = 0; i < image.data.length; i++) image.data[i] = (i % 4 === 3) ? 255 : Math.floor(Math.random() * 256);
  context.putImageData(image, 0, 0);
});
const png = await page.screenshot({ type: 'png', fullPage: true });
const pdf = await page.pdf({ printBackground: true });
return { png, pdf, small: Buffer.from('inline'), sizes: { png: png.length, pdf: pdf.length } };
"""


def blobs(url: str, socket_path: str, token_file: str) -> dict:
    """zuzu's full-page screenshot and PDF, which no longer fit in a reply, read back as blobs."""
    client = connect(url)
    problems = []
    got = call_run(client, BLOBS_CODE, {"taskId": "accept-blobs"})
    value = (got["value"] or {}).get("value") or {}
    run_id = got["meta"].get("id", "")
    sizes = value.get("sizes") or {}
    if got["meta"].get("status") != "ok":
        problems.append(f"blob run: {got['meta'].get('status')} {got['text'][:200]}")
    if sizes.get("png", 0) < 1_500_000:
        problems.append(f"the full-page PNG is {sizes.get('png')} bytes, under 1.5 MB")
    if value.get("small") != {"$neo": "bytes", "base64": "aW5saW5l"}:
        problems.append(f"small bytes did not stay inline: {value.get('small')}")
    read = {}
    for name, magic in (("png", b"\x89PNG\r\n\x1a\n"), ("pdf", b"%PDF-")):
        marker = value.get(name) or {}
        if marker.get("$neo") != "blob" or marker.get("size") != sizes.get(name):
            problems.append(f"{name} marker: {str(marker)[:120]}")
            continue
        raw = host_raw(socket_path, token_file, f"/v1/runs/{run_id}/blobs/{marker.get('id')}")
        status, _, body = raw if isinstance(raw, tuple) else (raw, {}, b"")
        if status != 200 or len(body) != marker["size"] or not body.startswith(magic):
            problems.append(f"{name} blob: {status}, {len(body)} bytes, starts {body[:8]!r}")
        read[name] = f"{marker.get('id')} {len(body)} B"
    text = call_run(client, "return 'x'.repeat(300 * 1024);", {"taskId": "accept-blobs"})
    error = ((text["value"] or {}).get("error") or {})
    if error.get("code") != "result_too_large" or "Buffer.from" not in error.get("message", ""):
        problems.append(f"large text: {text['meta'].get('status')} {str(error)[:200]}")
    return {
        "ok": not problems,
        "run": run_id,
        "blob": (value.get("png") or {}).get("id", ""),
        "problems": problems,
        "detail": f"run {run_id}: png {read.get('png')}, pdf {read.get('pdf')} read over the socket; "
        "small bytes inline; 300 KiB of text result_too_large",
    }


def main() -> None:
    command, args = sys.argv[1], sys.argv[2:]
    if command == "run":
        out = run(*args)
    elif command == "downloads":
        out = downloads(*args)
    elif command == "pdf-downloads":
        out = pdf_downloads(*args)
    elif command == "blobs":
        out = blobs(*args)
    elif command == "host":
        out = host(args[0], args[1], urllib.parse.quote(args[2], safe="/?=&"))
    elif command == "host-send":
        # socket, token file, method, path, JSON body
        out = host(args[0], args[1], urllib.parse.quote(args[3], safe="/?=&"), args[2], args[4])
    elif command == "describe":
        out = describe(args[0])
    elif command == "listeners":
        out = listeners()
    elif command == "probe":
        out = probe(args[0], int(args[1]), args[2])
    else:
        raise SystemExit(f"unknown command {command}")
    print(json.dumps(out))


if __name__ == "__main__":
    main()
