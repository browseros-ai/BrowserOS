/**
 * Marks for agents that @lobehub/icons does not carry.
 *
 * Vector marks are inlined rather than imported: the Chromium build emits only
 * app.js, app.css and the icon/ files listed in that target's BUILD.gn, so a
 * Vite-processed asset either throws during the build or trips the data: URL
 * guard in verify-chromium-build. Raster marks are referenced by their runtime
 * path under public/icon instead, which Vite copies through untouched.
 */

interface MarkProps {
  size?: number
}

/**
 * Hermes. Raster mark, so it ships as an icon/ resource rather than a path.
 *
 * Sized a little larger than the vector marks and rounded: it is a full-bleed
 * illustration rather than a glyph, so at glyph size it reads as a framed
 * thumbnail sitting inside the chip instead of as the chip's own icon.
 */
export function Hermes({ size = 24 }: MarkProps) {
  return (
    <img
      src="/icon/hermes.png"
      alt=""
      width={Math.round(size * 1.3)}
      height={Math.round(size * 1.3)}
      className="rounded-[7px] object-contain"
    />
  )
}

/**
 * OpenClaw. Silhouette of the mark the app ships, flattened to one colour.
 *
 * The eyes are cut out of the body with evenodd rather than painted over, so
 * the shape stays legible against any chip background, and the claws are kept
 * clear of the body so the outline does not read as one blob at chip size.
 */
export function OpenClaw({ size = 24 }: MarkProps) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 120 120"
      fill="currentColor"
      xmlns="http://www.w3.org/2000/svg"
      role="img"
      aria-label="OpenClaw"
    >
      <path
        fillRule="evenodd"
        clipRule="evenodd"
        d="M60 22c-19 0-30 16-30 35 0 19 12 35 30 43 18-8 30-24 30-43 0-19-11-35-30-35Zm-9 22a6 6 0 1 1 0 12 6 6 0 0 1 0-12Zm18 0a6 6 0 1 1 0 12 6 6 0 0 1 0-12Z"
      />
      <path d="M24 48c-13-6-19 3-15 12 4 9 14 6 19-3 3-5 1-8-4-9Z" />
      <path d="M96 48c13-6 19 3 15 12-4 9-14 6-19-3-3-5-1-8 4-9Z" />
      <path
        d="M48 24 38 10"
        stroke="currentColor"
        strokeWidth="7"
        strokeLinecap="round"
      />
      <path
        d="M72 24 82 10"
        stroke="currentColor"
        strokeWidth="7"
        strokeLinecap="round"
      />
    </svg>
  )
}
