# slimviewer

The Neo SDK image builds and runs slimviewer beside Xvnc. It streams the X11
screen as H.264 to a phone or desktop browser. The page draws frames with
WebCodecs and sends touch, keyboard and clipboard input back to Xvnc.

XDamage wakes the capture loop when the screen changes. The viewer acknowledges
drawn frames so the streamer can skip stale frames on a slow connection. Once
motion stops, the streamer sharpens the picture at full resolution.

## Run

```sh
slimviewer --display :1 --rfb 127.0.0.1:5900 \
  --listen 127.0.0.1:5910 --listen-view 127.0.0.1:5911
```

These are the SDK defaults. Run the binary as the Xvnc user in the same IPC
namespace. The host serves the page and bridges each authenticated WebSocket
to the appropriate loopback TCP port. See [Host integration](docs/integrate.md).

| Option | Default | Purpose |
| --- | --- | --- |
| `--display` | `:1` | X11 screen to capture |
| `--rfb` | `127.0.0.1:5900` | Xvnc input connection |
| `--listen` | `127.0.0.1:5910` | Owner stream with input |
| `--listen-view` | `127.0.0.1:5911` | View-only stream |
| `--max-viewers` | `8` | Spectator limit, or zero to disable spectators |
| `--encoder`, `--threads` | `openh264`, `0` | Compiled encoder and thread count (zero selects automatically) |
| `--fps`, `--crf`, `--crf-range` | `30`, `24`, `12` | Frame rate ceiling, starting quantizer and allowed increase |
| `--sharpen-qp`, `--sharpen-ms` | `18`, `250` | Still-frame quantizer and delay in milliseconds |
| `--no-cpu-guard`, `--full-size-only` | off | Disable CPU throttling feedback or adaptive scaling |
| `--preset` | `ultrafast` | Encoder preset for x264 builds |
| `--help`, `--version` | | Print options or version and exit |

## Build with Docker

From this directory, export the release binary, web assets and licence notices, or
run formatting, Clippy and tests for the default, x264-only and combined features.

```sh
docker buildx build --output type=local,dest=target/rootfs .
docker buildx build --target slimviewer-check --output type=cacheonly .
```

The release uses OpenH264. To build the full SDK image, run `../neo-server/docker/build.sh`.

## Build on Linux

Use Rust 1.98.0, Git, Make, a C/C++ toolchain and NASM. From this directory,
with write access to the install prefix, run

```sh
sh csrc/build-openh264.sh /opt/openh264 && OPENH264_ROOT=/opt/openh264 cargo build --release
```

The binary is `target/release/slimviewer`. This crate has its own Cargo workspace
and lockfile so SDK developers can build the other crates without the native
encoder toolchain. The image builds OpenH264 only. For an x264-only build,
install `libx264-dev` and use
`cargo build --release --locked --no-default-features --features x264`.

The crate uses AGPL-3.0-or-later. See [Licences](docs/licences.md) for the encoder
licences, bundled notices and H.264 patent note.
