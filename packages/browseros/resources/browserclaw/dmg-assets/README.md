# BrowserOS neo DMG appearance

Background art for the mounted install disk image. The macOS packaging step feeds these to `dmgbuild`, which positions the real app icon and the Applications alias on top of the art and bakes the window layout into the image's `.DS_Store`.

## Files

| File | Size | Purpose |
| --- | --- | --- |
| `background.png` | 640 x 420 | 1x background, matches the DMG window content size |
| `background@2x.png` | 1280 x 840 | Retina variant, picked up automatically by `dmgbuild` |

## Layout contract

The icons are placed by the build, not drawn into the art. Keep the arrow and copy clear of these points so the real icons land on them:

- App icon center: `(165, 190)`
- Applications alias center: `(475, 190)`
- Window content: `640 x 420`, icon size `128`

If you change the art, keep both sizes in sync and keep those two icon zones empty. The positions themselves live in `bos_build/steps/package/macos.py` (`DmgAppearance`); change them there and here together.

These are first-pass assets and are safe to replace with a designed version as long as the dimensions and the two icon zones stay the same.
