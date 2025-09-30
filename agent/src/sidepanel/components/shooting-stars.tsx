"use client";
import { cn } from "@/sidepanel/lib/utils";
import React, { useEffect, useState, useRef } from "react";

interface ShootingStar {
  id: number;
  x: number;
  y: number;
  angle: number;
  scale: number;
  speed: number;
  distance: number;
  color: string;
  trailColor: string;
}

interface ShootingStarsProps {
  minSpeed?: number;
  maxSpeed?: number;
  minDelay?: number;
  maxDelay?: number;
  starColor?: string;
  trailColor?: string;
  starWidth?: number;
  starHeight?: number;
  className?: string;
}

const getRandomStartPoint = () => {
  const side = Math.floor(Math.random() * 4);
  const offset = Math.random() * window.innerWidth;

  switch (side) {
    case 0:
      return { x: offset, y: 0, angle: 45 };
    case 1:
      return { x: window.innerWidth, y: offset, angle: 135 };
    case 2:
      return { x: offset, y: window.innerHeight, angle: 225 };
    case 3:
      return { x: 0, y: offset, angle: 315 };
    default:
      return { x: 0, y: 0, angle: 45 };
  }
};
export const ShootingStars: React.FC<ShootingStarsProps> = ({
  minSpeed = 10,
  maxSpeed = 30,
  minDelay = 1200,
  maxDelay = 4200,
  starColor = "#9E00FF",
  trailColor = "#2EB9DF",
  starWidth = 10,
  starHeight = 1,
  className,
}) => {
  const [star, setStar] = useState<ShootingStar | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);

  useEffect(() => {
    const createStar = () => {
      const { x, y, angle } = getRandomStartPoint();

      // Bright galaxy color combinations for shooting stars
      const colorCombinations = [
        { starColor: '#FF0080', trailColor: '#8000FF' }, // Magenta to purple
        { starColor: '#00FFFF', trailColor: '#FF00FF' }, // Cyan to magenta
        { starColor: '#FFFF00', trailColor: '#FF8000' }, // Yellow to orange
        { starColor: '#FF0000', trailColor: '#FF8000' }, // Red to orange
        { starColor: '#00FF00', trailColor: '#0080FF' }, // Green to blue
        { starColor: '#8000FF', trailColor: '#FF0080' }, // Purple to pink
        { starColor: '#FF8000', trailColor: '#FFFF00' }, // Orange to yellow
        { starColor: '#0080FF', trailColor: '#00FFFF' }, // Blue to cyan
        { starColor: '#FF0080', trailColor: '#00FFFF' }, // Pink to cyan
        { starColor: '#80FF00', trailColor: '#FF8000' }, // Chartreuse to orange
      ];

      const colorCombo = colorCombinations[Math.floor(Math.random() * colorCombinations.length)];

      const newStar: ShootingStar = {
        id: Date.now(),
        x,
        y,
        angle,
        scale: 1,
        speed: Math.random() * (maxSpeed - minSpeed) + minSpeed,
        distance: 0,
        color: colorCombo.starColor,
        trailColor: colorCombo.trailColor,
      };
      setStar(newStar);

      const randomDelay = Math.random() * (maxDelay - minDelay) + minDelay;
      setTimeout(createStar, randomDelay);
    };

    createStar();

    return () => {};
  }, [minSpeed, maxSpeed, minDelay, maxDelay]);

  useEffect(() => {
    const moveStar = () => {
      if (star) {
        setStar((prevStar) => {
          if (!prevStar) return null;
          const newX =
            prevStar.x +
            prevStar.speed * Math.cos((prevStar.angle * Math.PI) / 180);
          const newY =
            prevStar.y +
            prevStar.speed * Math.sin((prevStar.angle * Math.PI) / 180);
          const newDistance = prevStar.distance + prevStar.speed;
          const newScale = 1 + newDistance / 100;
          if (
            newX < -20 ||
            newX > window.innerWidth + 20 ||
            newY < -20 ||
            newY > window.innerHeight + 20
          ) {
            return null;
          }
          return {
            ...prevStar,
            x: newX,
            y: newY,
            distance: newDistance,
            scale: newScale,
          };
        });
      }
    };

    const animationFrame = requestAnimationFrame(moveStar);
    return () => cancelAnimationFrame(animationFrame);
  }, [star]);

  return (
    <svg
      ref={svgRef}
      className={cn("w-full h-full absolute inset-0", className)}
    >
      {star && (
        <rect
          key={star.id}
          x={star.x}
          y={star.y}
          width={starWidth * star.scale}
          height={starHeight}
          fill={`url(#gradient-${star.id})`}
          transform={`rotate(${star.angle}, ${
            star.x + (starWidth * star.scale) / 2
          }, ${star.y + starHeight / 2})`}
        />
      )}
      <defs>
        {star && (
          <linearGradient id={`gradient-${star.id}`} x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" style={{ stopColor: star.trailColor, stopOpacity: 0 }} />
            <stop
              offset="100%"
              style={{ stopColor: star.color, stopOpacity: 1 }}
            />
          </linearGradient>
        )}
      </defs>
    </svg>
  );
};
