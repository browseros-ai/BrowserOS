"use client";

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

export type ActionSwapRollButtonProps = Omit<ActionSwapButtonProps, "animation">;
export type ActionSwapRollTextProps = Omit<ActionSwapTextProps, "animation">;
export type ActionSwapRollIconProps = Omit<ActionSwapIconProps, "animation">;

export function ActionSwapRollButton(props: ActionSwapRollButtonProps) {
  return <ActionSwapButton {...props} animation="roll" />;
}

export function ActionSwapRollText(props: ActionSwapRollTextProps) {
  return <ActionSwapText {...props} animation="roll" />;
}

export function ActionSwapRollIcon(props: ActionSwapRollIconProps) {
  return <ActionSwapIcon {...props} animation="roll" />;
}
