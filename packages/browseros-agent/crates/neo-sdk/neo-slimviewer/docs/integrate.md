# Host integration

The Neo SDK image installs `/usr/local/bin/slimviewer`. The supervisor starts it
as `neo` after the X11 socket for display `:1` appears, using the same UID and
IPC namespace as Xvnc. It restarts the streamer five seconds after exit and
stops it on shutdown. Streaming is on by default. Set `NEO_SLIMVIEWER=0` to
disable it. The supervisor also skips startup if the binary is missing.

| Installed path | Contents |
| --- | --- |
| `/usr/share/slimviewer/slimviewer.html` | WebCodecs viewer |
| `/usr/share/slimviewer/ui.js` | Viewer state and paste adapter for host controls |
| `/usr/share/slimviewer/RUNTIME-DEPS` | Required runtime packages |
| `/usr/share/doc/slimviewer/THIRD-PARTY-LICENSES` | Bundled notices |
| `/usr/share/doc/slimviewer/OPENH264-SOURCE` | OpenH264 version tag and Git revision |

## Build the image

From the slimviewer crate, run `../neo-server/docker/build.sh --platform linux/arm64`
or use `linux/amd64`. The script builds this crate's Dockerfile for that CPU and passes
its release files to the SDK image. The SDK checks the packages in `RUNTIME-DEPS` and
records the binary hash and OpenH264 revision in `/usr/share/neo-sdk/provenance.json`.

## Serve the viewer

1. Serve the HTML and adapter over HTTPS (or localhost for development). The
   page opens a WebSocket on the same origin at `/slim` by default. To use
   `/screen/stream`, serve `slimviewer.html?path=screen/stream`.
2. Authenticate each WebSocket and choose its upstream on the server. Forward
   bytes to `127.0.0.1:5910` for the owner or `127.0.0.1:5911` for a spectator.
   Keep both ports on loopback. Only the owner port accepts input. A new owner
   replaces the previous one. Xvnc uses `127.0.0.1:5900` for RFB.
3. For an iframe, set `window.SLIM_PARENT_ORIGINS` to an array of trusted
   parent origins before the viewer script runs. The page sends `host-viewer`
   state and `host-layout` messages to these origins. The parent can request
   layout with `host-sizing` and provide editable-field bounds with `host-keyboard`.
4. Supply your own `host-*.js` files if you use a takeover pill. Its
   `host-clipboard.js` can re-export `pasteFromPill` from `./ui.js`. Invoke
   paste directly from the user's tap so the browser grants clipboard access.

The page also exposes `window.slimviewer` with `connected`, `viewOnly` and
`state`, the `ready`, `state` and `input` events, and `paste()`, `type(text)`
and `key(name)` methods.
