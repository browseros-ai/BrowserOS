"use client";
import { cn } from "@/sidepanel/lib/utils";
import React, {
  useState,
  useEffect,
  useRef,
  RefObject,
  useCallback,
} from "react";

interface StarProps {
  x: number;
  y: number;
  radius: number;
  opacity: number;
  twinkleSpeed: number | null;
  color: string;
}

interface StarBackgroundProps {
  starDensity?: number;
  allStarsTwinkle?: boolean;
  twinkleProbability?: number;
  minTwinkleSpeed?: number;
  maxTwinkleSpeed?: number;
  className?: string;
}

export const StarsBackground: React.FC<StarBackgroundProps> = ({
  starDensity = 0.00015,
  allStarsTwinkle = true,
  twinkleProbability = 0.7,
  minTwinkleSpeed = 0.5,
  maxTwinkleSpeed = 1,
  className,
}) => {
  const [stars, setStars] = useState<StarProps[]>([]);
  const canvasRef: RefObject<HTMLCanvasElement> =
    useRef<HTMLCanvasElement>(null);

  const generateStars = useCallback(
    (width: number, height: number): StarProps[] => {
      const area = width * height;
      const numStars = Math.floor(area * starDensity);

      // Bright galaxy color palette
      const starColors = [
        '#FFFFFF', // Pure White
        '#FFE4E1', // Misty Rose
        '#E6E6FA', // Lavender
        '#F0E68C', // Khaki
        '#DDA0DD', // Plum
        '#98FB98', // Pale Green
        '#F0F8FF', // Alice Blue
        '#FFB6C1', // Light Pink
        '#87CEEB', // Sky Blue
        '#D3D3D3', // Light Gray
        '#FFF8DC', // Cornsilk
        '#B0E0E6', // Powder Blue
        '#FF69B4', // Hot Pink
        '#00CED1', // Dark Turquoise
        '#32CD32', // Lime Green
        '#FFD700', // Gold
        '#FF4500', // Orange Red
        '#9370DB', // Medium Purple
        '#00FA9A', // Medium Spring Green
        '#FF1493', // Deep Pink
      ];

      return Array.from({ length: numStars }, () => {
        const shouldTwinkle =
          allStarsTwinkle || Math.random() < twinkleProbability;
        return {
          x: Math.random() * width,
          y: Math.random() * height,
          radius: Math.random() * 0.4 + 0.4, // Medium stars
          opacity: Math.random() * 0.4 + 0.7, // Brighter stars
          twinkleSpeed: shouldTwinkle
            ? minTwinkleSpeed +
              Math.random() * (maxTwinkleSpeed - minTwinkleSpeed)
            : null,
          color: starColors[Math.floor(Math.random() * starColors.length)],
        };
      });
    },
    [
      starDensity,
      allStarsTwinkle,
      twinkleProbability,
      minTwinkleSpeed,
      maxTwinkleSpeed,
    ]
  );

  useEffect(() => {
    const updateStars = () => {
      if (canvasRef.current) {
        const canvas = canvasRef.current;
        const ctx = canvas.getContext("2d");
        if (!ctx) return;

        const { width, height } = canvas.getBoundingClientRect();
        canvas.width = width;
        canvas.height = height;
        setStars(generateStars(width, height));
      }
    };

    updateStars();

    const resizeObserver = new ResizeObserver(updateStars);
    if (canvasRef.current) {
      resizeObserver.observe(canvasRef.current);
    }

    return () => {
      if (canvasRef.current) {
        resizeObserver.unobserve(canvasRef.current);
      }
    };
  }, [
    starDensity,
    allStarsTwinkle,
    twinkleProbability,
    minTwinkleSpeed,
    maxTwinkleSpeed,
    generateStars,
  ]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let animationFrameId: number;

  const hexToRgb = (hex: string): { r: number; g: number; b: number } => {
    const result = /^#?([a-f\d]{2})([a-f\d]{2})([a-f\d]{2})$/i.exec(hex);
    return result
      ? {
          r: parseInt(result[1], 16),
          g: parseInt(result[2], 16),
          b: parseInt(result[3], 16),
        }
      : { r: 255, g: 255, b: 255 };
  };

  const render = () => {
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    stars.forEach((star) => {
      ctx.beginPath();
      ctx.arc(star.x, star.y, star.radius, 0, Math.PI * 2);
      const rgb = hexToRgb(star.color);
      ctx.fillStyle = `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, ${star.opacity})`;
      ctx.fill();

      if (star.twinkleSpeed !== null) {
        star.opacity =
          0.5 +
          Math.abs(Math.sin((Date.now() * 0.001) / star.twinkleSpeed) * 0.5);
      }
    });

    animationFrameId = requestAnimationFrame(render);
  };

    render();

    return () => {
      cancelAnimationFrame(animationFrameId);
    };
  }, [stars]);

  return (
    <canvas
      ref={canvasRef}
      className={cn("h-full w-full absolute inset-0", className)}
    />
  );
};
