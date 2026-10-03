# Windows install identity fixture

`chromium_install_modes.h.txt` is the byte-for-byte upstream header from Chromium
155.0.8059.26, commit `16c3e55476d3564bea713314b2fff638749ce3e6`:

- [Original header](https://chromium.googlesource.com/chromium/src/+/16c3e55476d3564bea713314b2fff638749ce3e6/chrome/install_static/chromium_install_modes.h)
- Git blob: `a6d969df56d13d94eaa968934f909d6fd23b5f2f`
- [Tracing interface definition](https://chromium.googlesource.com/chromium/src/+/16c3e55476d3564bea713314b2fff638749ce3e6/chrome/windows_services/elevated_tracing_service/tracing_service_idl.idl#57)

The existing product identity assertions apply the actual BrowserOS patch to this
fixture using Git in a temporary directory. This covers complete shared COM
initializers even when they fall outside the patch's context lines. The fixture
must match the patch's preimage blob; mismatches fail with a refresh instruction.
No network or Chromium checkout is used during validation.

When upgrading the patch base, download the new upstream header with Gitiles
`?format=TEXT`, base64-decode it without newline conversion, and update these
source references. Verify any changed IID against the corresponding upstream
IDL before updating expectations. Do not replace upstream values with product
CLSIDs: the products register separate classes implementing shared interfaces.

Chromium 155 uses `E0B03E2D-7682-4D83-B9FF-4574AF720500` for
`ISystemTraceSessionChromium`. The prior IID
`A3FD580A-FFD4-4075-9174-75D0B199D3CB` remains in `kOldTracingServiceIids`
for cleanup, not as the active interface. The upstream license is in `LICENSE`.
