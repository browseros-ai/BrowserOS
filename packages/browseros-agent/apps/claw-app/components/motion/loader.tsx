"use client";
// beui.dev/components/motion/loader

import { motion, useReducedMotion } from "motion/react";
import { useEffect, useId, useState } from "react";
import { EASE_IN_OUT } from "@/lib/ease";
import { cn } from "@/lib/utils";

export type LoaderVariant =
  | "spinner"
  | "dots"
  | "bars"
  | "dot-matrix"
  | "dither"
  | "ascii"
  | "ascii-line"
  | "ascii-braille"
  | "ascii-blocks"
  | "ascii-bounce"
  | "morph"
  | "comet"
  | "scramble"
  | "metaballs"
  | "newton"
  | "helix"
  | "percent";

// Terminal-style frame sets — the loaders CLI AI agents cycle through.
const ASCII_SETS: Record<string, string[]> = {
  ascii: ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
  "ascii-line": ["|", "/", "-", "\\"],
  "ascii-braille": ["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
  "ascii-blocks": ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█", "▇", "▆", "▅", "▄", "▃", "▂"],
  "ascii-bounce": ["⠁", "⠂", "⠄", "⡀", "⢀", "⠠", "⠐", "⠈"],
};

export interface LoaderProps {
  /** Which animation to render. */
  variant?: LoaderVariant;
  /** Base square size in px. Everything scales from this. */
  size?: number;
  /** Seconds per animation cycle. */
  speed?: number;
  /** Accessible label announced to screen readers. */
  label?: string;
  className?: string;
}

// Reduced motion keeps a calm opacity pulse and drops every transform.
const REDUCED = {
  animate: { opacity: [1, 0.4, 1] },
  transition: { duration: 1.4, ease: EASE_IN_OUT, repeat: Infinity },
};

export function Loader({
  variant = "spinner",
  size = 32,
  speed = 1,
  label = "Loading",
  className,
}: LoaderProps) {
  const reduce = useReducedMotion() ?? false;

  return (
    <span
      role="status"
      aria-label={label}
      className={cn(
        "inline-flex items-center justify-center text-foreground",
        className,
      )}
    >
      {variant === "spinner" && <Spinner size={size} speed={speed} reduce={reduce} />}
      {variant === "dots" && <Dots size={size} speed={speed} reduce={reduce} />}
      {variant === "bars" && <Bars size={size} speed={speed} reduce={reduce} />}
      {variant === "dot-matrix" && (
        <DotMatrix size={size} speed={speed} reduce={reduce} />
      )}
      {variant === "dither" && <Dither size={size} speed={speed} reduce={reduce} />}
      {ASCII_SETS[variant] && (
        <Ascii frames={ASCII_SETS[variant]} size={size} speed={speed} reduce={reduce} />
      )}
      {variant === "morph" && <Morph size={size} speed={speed} reduce={reduce} />}
      {variant === "comet" && <Comet size={size} speed={speed} reduce={reduce} />}
      {variant === "scramble" && (
        <Scramble size={size} speed={speed} reduce={reduce} />
      )}
      {variant === "metaballs" && (
        <Metaballs size={size} speed={speed} reduce={reduce} />
      )}
      {variant === "newton" && <Newton size={size} speed={speed} reduce={reduce} />}
      {variant === "helix" && <Helix size={size} speed={speed} reduce={reduce} />}
      {variant === "percent" && (
        <Percent size={size} speed={speed} reduce={reduce} />
      )}
      <span className="sr-only">{label}</span>
    </span>
  );
}

interface PartProps {
  size: number;
  speed: number;
  reduce: boolean;
}

function Spinner({ size, speed, reduce }: PartProps) {
  const stroke = Math.max(2, size * 0.09);
  const r = (size - stroke) / 2;
  return (
    <motion.svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      animate={reduce ? REDUCED.animate : { rotate: 360 }}
      transition={
        reduce
          ? REDUCED.transition
          : { duration: speed, ease: "linear", repeat: Infinity }
      }
    >
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke="currentColor"
        strokeOpacity={0.2}
        strokeWidth={stroke}
      />
      <path
        d={`M ${size / 2} ${size / 2 - r} A ${r} ${r} 0 0 1 ${size / 2 + r} ${size / 2}`}
        fill="none"
        stroke="currentColor"
        strokeWidth={stroke}
        strokeLinecap="round"
      />
    </motion.svg>
  );
}

function Dots({ size, speed, reduce }: PartProps) {
  const dot = size * 0.24;
  return (
    <span className="flex items-center" style={{ gap: size * 0.14 }}>
      {[0, 1, 2].map((i) => (
        <motion.span
          key={i}
          className="rounded-full bg-current"
          style={{ width: dot, height: dot }}
          animate={
            reduce
              ? { opacity: [0.4, 1, 0.4] }
              : { y: [0, -size * 0.3, 0], opacity: [0.5, 1, 0.5] }
          }
          transition={{
            duration: speed,
            ease: EASE_IN_OUT,
            repeat: Infinity,
            delay: i * speed * 0.16,
          }}
        />
      ))}
    </span>
  );
}

function Ascii({
  frames,
  size,
  speed,
  reduce,
}: PartProps & { frames: string[] }) {
  const [frame, setFrame] = useState(0);
  useEffect(() => {
    // Reduced motion slows the cycle rather than stopping it — it's a glyph
    // swap, not on-screen movement.
    const step = ((reduce ? speed * 2.5 : speed) / frames.length) * 1000;
    const id = setInterval(
      () => setFrame((f) => (f + 1) % frames.length),
      step,
    );
    return () => clearInterval(id);
  }, [frames.length, speed, reduce]);

  return (
    <span
      className="font-mono leading-none tabular-nums"
      style={{ fontSize: size, lineHeight: 1 }}
    >
      {frames[frame % frames.length]}
    </span>
  );
}

// Each shape is sampled at the same number of points and emitted as an SVG
// path with identical command structure, so framer tweens the `d` attribute
// point-to-point — a real morph, not a snap. (clip-path polygon strings don't
// interpolate reliably in framer, which left the shapes broken.)
const MORPH_POINTS = 24;

function ngonRadius(ang: number, n: number, phase = 0) {
  const seg = (2 * Math.PI) / n;
  const a = ang - phase;
  const local = (((a % seg) + seg) % seg) - seg / 2;
  return Math.cos(Math.PI / n) / Math.cos(local);
}

function morphPath(radiusAt: (ang: number) => number) {
  const parts: string[] = [];
  for (let i = 0; i < MORPH_POINTS; i++) {
    const ang = (i / MORPH_POINTS) * 2 * Math.PI - Math.PI / 2;
    const r = Math.min(1.05, radiusAt(ang));
    const x = (50 + Math.cos(ang) * 46 * r).toFixed(2);
    const y = (50 + Math.sin(ang) * 46 * r).toFixed(2);
    parts.push(`${i === 0 ? "M" : "L"}${x} ${y}`);
  }
  return `${parts.join(" ")} Z`;
}

const MORPH_PATHS = [
  morphPath(() => 1), // circle
  morphPath((a) => ngonRadius(a, 4, Math.PI / 4)), // square
  morphPath((a) => ngonRadius(a, 3)), // triangle
  morphPath((a) => ngonRadius(a, 6)), // hexagon
  morphPath((a) => ngonRadius(a, 4)), // diamond
];

// Each shape appears twice in a row so it fully forms and HOLDS before the
// next morph. Even keyframe spacing then alternates hold / morph segments.
const MORPH_SEQ = [...MORPH_PATHS.flatMap((p) => [p, p]), MORPH_PATHS[0]];
// Rotation and scale only change across the morph segments, staying put on the
// holds, so a settled shape sits still.
const MORPH_ROT = [0, 0, 72, 72, 144, 144, 216, 216, 288, 288, 360];
const MORPH_SCALE = [1, 1, 0.88, 0.88, 1, 1, 0.88, 0.88, 1, 1, 1];

function Morph({ size, speed, reduce }: PartProps) {
  return (
    <motion.svg
      width={size}
      height={size}
      viewBox="0 0 100 100"
      role="img"
      animate={
        reduce
          ? { opacity: [1, 0.4, 1] }
          : { rotate: MORPH_ROT, scale: MORPH_SCALE }
      }
      transition={
        reduce
          ? { duration: 1.4, ease: EASE_IN_OUT, repeat: Infinity }
          : { duration: speed * 5, ease: EASE_IN_OUT, repeat: Infinity }
      }
    >
      <title>Loading</title>
      <motion.path
        fill="currentColor"
        d={MORPH_PATHS[0]}
        animate={reduce ? undefined : { d: MORPH_SEQ }}
        transition={
          reduce
            ? undefined
            : { duration: speed * 5, ease: EASE_IN_OUT, repeat: Infinity }
        }
      />
    </motion.svg>
  );
}

const COMET_TRAIL = [0, 1, 2, 3, 4, 5];

function Comet({ size, speed, reduce }: PartProps) {
  const head = size * 0.2;
  const r = size / 2 - head / 2;
  return (
    <span className="relative" style={{ width: size, height: size }}>
      <motion.span
        className="absolute inset-0"
        animate={reduce ? REDUCED.animate : { rotate: 360 }}
        transition={
          reduce
            ? REDUCED.transition
            : { duration: speed, ease: "linear", repeat: Infinity }
        }
      >
        {COMET_TRAIL.map((i) => {
          const scale = 1 - i * 0.13;
          const sz = head * scale;
          return (
            <span
              key={i}
              className="absolute top-1/2 left-1/2 rounded-full bg-current"
              style={{
                width: sz,
                height: sz,
                marginLeft: -sz / 2,
                marginTop: -sz / 2,
                opacity: 1 - i * 0.16,
                transform: `rotate(${-i * 15}deg) translateY(${-r}px)`,
              }}
            />
          );
        })}
      </motion.span>
    </span>
  );
}

const SCRAMBLE_TARGET = "LOADING";
const SCRAMBLE_GLYPHS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789<>/*#@";

function Scramble({ size, speed, reduce }: PartProps) {
  const [text, setText] = useState(SCRAMBLE_TARGET);
  useEffect(() => {
    if (reduce) {
      setText(SCRAMBLE_TARGET);
      return;
    }
    let tick = 0;
    const total = SCRAMBLE_TARGET.length + 4;
    const id = setInterval(
      () => {
        const reveal = tick % total;
        let s = "";
        for (let i = 0; i < SCRAMBLE_TARGET.length; i++) {
          s +=
            i < reveal
              ? SCRAMBLE_TARGET[i]
              : SCRAMBLE_GLYPHS[
                  Math.floor(Math.random() * SCRAMBLE_GLYPHS.length)
                ];
        }
        setText(s);
        tick++;
      },
      (speed / SCRAMBLE_TARGET.length) * 1000 * 0.55,
    );
    return () => clearInterval(id);
  }, [speed, reduce]);

  return (
    <span
      className="font-mono font-medium tracking-[0.2em] tabular-nums"
      style={{ fontSize: size * 0.42 }}
    >
      {text}
    </span>
  );
}

function Metaballs({ size, speed, reduce }: PartProps) {
  const id = useId().replace(/:/g, "");
  return (
    <svg width={size} height={size} viewBox="0 0 100 100" role="img">
      <title>Loading</title>
      <defs>
        <filter id={id}>
          <feGaussianBlur in="SourceGraphic" stdDeviation="5" result="b" />
          <feColorMatrix
            in="b"
            values="1 0 0 0 0  0 1 0 0 0  0 0 1 0 0  0 0 0 20 -8"
          />
        </filter>
      </defs>
      <g filter={`url(#${id})`} fill="currentColor">
        <motion.circle
          cy="50"
          r="15"
          animate={reduce ? { opacity: [0.4, 1, 0.4] } : { cx: [30, 70, 30] }}
          transition={{ duration: speed * 1.6, ease: EASE_IN_OUT, repeat: Infinity }}
          cx={reduce ? 40 : 30}
        />
        <motion.circle
          cy="50"
          r="15"
          animate={reduce ? { opacity: [0.4, 1, 0.4] } : { cx: [70, 30, 70] }}
          transition={{ duration: speed * 1.6, ease: EASE_IN_OUT, repeat: Infinity }}
          cx={reduce ? 60 : 70}
        />
      </g>
    </svg>
  );
}

const NEWTON_BALLS = [0, 1, 2, 3, 4];

function Newton({ size, speed, reduce }: PartProps) {
  const d = size * 0.2;
  const out = d * 1.1;
  // Only the end balls move: the left slides out and back on the first half,
  // then the right on the second half — the impact appears to jump the three
  // still middle balls. Pure horizontal slide, no swing, no strings.
  const moves: Record<number, { x: number[]; times: number[] }> = {
    0: { x: [0, -out, 0, 0], times: [0, 0.28, 0.5, 1] },
    4: { x: [0, 0, out, 0], times: [0, 0.5, 0.78, 1] },
  };

  return (
    <span className="flex items-center justify-center" style={{ height: d }}>
      {NEWTON_BALLS.map((i) => {
        const move = moves[i];
        return (
          <motion.span
            key={i}
            className="rounded-full bg-current"
            style={{ width: d, height: d }}
            animate={reduce || !move ? undefined : { x: move.x }}
            transition={
              reduce || !move
                ? undefined
                : {
                    duration: speed * 1.5,
                    ease: EASE_IN_OUT,
                    repeat: Infinity,
                    times: move.times,
                  }
            }
          />
        );
      })}
    </span>
  );
}

function Helix({ size, speed, reduce }: PartProps) {
  const rows = 7;
  const dot = size * 0.14;
  const amp = size * 0.32;
  return (
    <span className="relative" style={{ width: size, height: size }}>
      {Array.from({ length: rows }, (_, r) => {
        const top = (r / (rows - 1)) * (size - dot);
        const delay = (r / rows) * speed;
        return (
          <span key={`row-${top}`}>
            <motion.span
              className="absolute rounded-full bg-current"
              style={{ width: dot, height: dot, left: size / 2 - dot / 2, top }}
              animate={
                reduce
                  ? { opacity: [0.4, 1, 0.4] }
                  : {
                      x: [amp, -amp, amp],
                      scale: [1, 0.5, 1],
                      opacity: [1, 0.45, 1],
                    }
              }
              transition={{
                duration: speed,
                ease: EASE_IN_OUT,
                repeat: Infinity,
                delay,
              }}
            />
            <motion.span
              className="absolute rounded-full bg-current"
              style={{ width: dot, height: dot, left: size / 2 - dot / 2, top }}
              animate={
                reduce
                  ? { opacity: [0.4, 1, 0.4] }
                  : {
                      x: [-amp, amp, -amp],
                      scale: [0.5, 1, 0.5],
                      opacity: [0.45, 1, 0.45],
                    }
              }
              transition={{
                duration: speed,
                ease: EASE_IN_OUT,
                repeat: Infinity,
                delay,
              }}
            />
          </span>
        );
      })}
    </span>
  );
}

function Percent({ size, speed, reduce }: PartProps) {
  const [p, setP] = useState(0);
  useEffect(() => {
    const dur = (reduce ? speed * 2 : speed) * 1000;
    const start = { t: 0 };
    const tickMs = 40;
    const id = setInterval(() => {
      start.t += tickMs;
      const next = Math.min(100, Math.round((start.t / dur) * 100));
      setP(next);
      if (next >= 100) start.t = 0;
    }, tickMs);
    return () => clearInterval(id);
  }, [speed, reduce]);

  return (
    <span
      className="flex flex-col items-center"
      style={{ gap: size * 0.14, width: size * 1.4 }}
    >
      <span
        className="font-mono font-medium tabular-nums"
        style={{ fontSize: size * 0.42, lineHeight: 1 }}
      >
        {p}%
      </span>
      <span
        className="w-full overflow-hidden rounded-full bg-current/15"
        style={{ height: Math.max(3, size * 0.1) }}
      >
        <span
          className="block h-full rounded-full bg-current"
          style={{ width: `${p}%` }}
        />
      </span>
    </span>
  );
}

function Bars({ size, speed, reduce }: PartProps) {
  const bar = size * 0.16;
  return (
    <span className="flex items-center" style={{ gap: size * 0.1, height: size }}>
      {[0, 1, 2, 3].map((i) => (
        <motion.span
          key={i}
          className="rounded-full bg-current"
          style={{ width: bar, height: size, originY: 1 }}
          animate={
            reduce ? { opacity: [0.4, 1, 0.4] } : { scaleY: [0.3, 1, 0.3] }
          }
          transition={{
            duration: speed,
            ease: EASE_IN_OUT,
            repeat: Infinity,
            delay: i * speed * 0.12,
          }}
        />
      ))}
    </span>
  );
}

function DotMatrix({ size, speed, reduce }: PartProps) {
  const n = 3;
  const gap = size * 0.14;
  const dot = (size - gap * (n - 1)) / n;
  const cells = Array.from({ length: n * n }, (_, idx) => idx);
  return (
    <span
      className="grid"
      style={{
        gap,
        gridTemplateColumns: `repeat(${n}, ${dot}px)`,
      }}
    >
      {cells.map((idx) => {
        const x = idx % n;
        const y = Math.floor(idx / n);
        // Diagonal wave: cells light in order of their distance from the corner.
        const delay = ((x + y) / (2 * (n - 1))) * speed;
        return (
          <motion.span
            key={idx}
            className="rounded-full bg-current"
            style={{ width: dot, height: dot }}
            animate={
              reduce
                ? { opacity: [0.3, 1, 0.3] }
                : { opacity: [0.2, 1, 0.2], scale: [0.7, 1, 0.7] }
            }
            transition={{
              duration: speed,
              ease: EASE_IN_OUT,
              repeat: Infinity,
              delay,
            }}
          />
        );
      })}
    </span>
  );
}

// Ordered Bayer 4x4 matrix — the classic dithering threshold pattern. Cells
// light in this order, so the fill shimmers like a dissolving halftone.
const BAYER_4 = [
  0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5,
];

function Dither({ size, speed, reduce }: PartProps) {
  const n = 4;
  const gap = Math.max(1, size * 0.05);
  const cell = (size - gap * (n - 1)) / n;
  return (
    <span
      className="grid"
      style={{ gap, gridTemplateColumns: `repeat(${n}, ${cell}px)` }}
    >
      {BAYER_4.map((order, idx) => (
        <motion.span
          // biome-ignore lint/suspicious/noArrayIndexKey: fixed matrix cells, order never changes
          key={idx}
          className="bg-current"
          style={{ width: cell, height: cell }}
          animate={reduce ? { opacity: [0.3, 1, 0.3] } : { opacity: [0.1, 1, 0.1] }}
          transition={{
            duration: speed,
            ease: EASE_IN_OUT,
            repeat: Infinity,
            delay: (order / BAYER_4.length) * speed,
          }}
        />
      ))}
    </span>
  );
}
