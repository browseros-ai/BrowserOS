"use client";
// beui.dev/components/motion/action-swap

import {
  ActionSwapButton,
  ActionSwapIcon,
  ActionSwapText,
  type ActionSwapButtonProps,
  type ActionSwapIconProps,
  type ActionSwapTextProps,
} from "./action-swap";

export type {
  ActionSwapButtonSize,
  ActionSwapButtonVariant,
  ActionSwapItem,
} from "./action-swap";

export type ActionSwapBlurButtonProps = Omit<ActionSwapButtonProps, "animation">;
export type ActionSwapBlurTextProps = Omit<ActionSwapTextProps, "animation">;
export type ActionSwapBlurIconProps = Omit<ActionSwapIconProps, "animation">;

export function ActionSwapBlurButton(props: ActionSwapBlurButtonProps) {
  return <ActionSwapButton {...props} animation="blur" />;
}

export function ActionSwapBlurText(props: ActionSwapBlurTextProps) {
  return <ActionSwapText {...props} animation="blur" />;
}

export function ActionSwapBlurIcon(props: ActionSwapBlurIconProps) {
  return <ActionSwapIcon {...props} animation="blur" />;
}
