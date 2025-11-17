import { Platform } from 'react-native';

export const isLiquidGlassSupported = (): boolean => {
  // Liquid glass is only supported on iOS 15+
  return Platform.OS === 'ios' && parseInt(Platform.Version as string, 10) >= 15;
};
