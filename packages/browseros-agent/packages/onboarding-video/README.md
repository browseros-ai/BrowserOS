# @browseros/onboarding-video

Remotion compositions for the BrowserClaw cockpit first-run motion demo.

## Usage

**Runtime (default)**: `apps/claw-app` imports `FirstRunDemo` and renders it via `@remotion/player` inside the cockpit's first-run block. Composition source stays hot-swappable during development.

**Offline render (follow-up)**: to ship a pre-rendered WebM and drop the `@remotion/player` runtime cost, run:

```bash
bunx remotion render src/index.ts FirstRunDemo out/first-run-demo.webm \
  --codec vp9 --width 1600 --height 900
bunx remotion still src/index.ts FirstRunDemo out/first-run-demo-poster.png \
  --frame 0
```

Then copy the outputs into `apps/claw-app/public/onboarding/` and swap the `<Player>` element in `FirstRunVideo.tsx` for a native `<video>`.

## Composition

- `FirstRunDemo`, 20s at 30fps (600 frames), 16:9 (native 1600x900).
- Five scenes (`src/scenes/`): cockpit, pan, prompt, activity, loop.
- Palette tokens mirror the claw-app cockpit CSS variables (`src/palette.ts`).
- Timing constants centralised in `src/timing.ts`.

## Design constraints (from `remotion-best-practices`)

- All motion via `useCurrentFrame()` + `interpolate()`. No CSS transitions or Tailwind animation classes; those do not render.
- Individual CSS transform properties (`scale`, `translate`, `rotate`) in style objects so Remotion Studio can edit inline.
- Assets under `public/` at the project root, referenced via `staticFile()` when needed.
