# Licences

The slimviewer crate and its web viewer are licensed under AGPL-3.0-or-later.
The full text is in the repository's [LICENSE](../../../../../../LICENSE).

| Encoder | Licence | Build |
| --- | --- | --- |
| OpenH264 2.7.0 | BSD-2-Clause | Default and sole encoder in the Neo SDK image. The build script verifies commit `1bca62311e447fd9c39fc93de2c4b96cd5ce971c` before building the static library. |
| x264 | GPL-2.0-or-later | Opt-in Cargo feature `x264`. An x264-enabled binary links the system library. |

The image bundles the OpenH264 source revision and BSD notice, locked Rust
dependency notices, and runtime notices in
`/usr/share/doc/slimviewer/THIRD-PARTY-LICENSES`. Retain them when
redistributing the binary. `/usr/share/slimviewer/RUNTIME-DEPS` lists its
system libraries.

H.264 patent rights are separate from these copyright licences. Cisco's
patent coverage applies to its distributed binary under its terms. This SDK
builds OpenH264 from source, so that binary coverage does not apply.
See https://www.openh264.org/faq.html for Cisco's terms.
