/**
 * Performance Monitoring & Optimization Utilities
 */

import { InteractionManager, Platform } from 'react-native';

/**
 * Defer heavy operations until after interactions complete
 */
export const runAfterInteractions = (callback: () => void): void => {
  InteractionManager.runAfterInteractions(() => {
    callback();
  });
};

/**
 * Debounce function calls
 */
export const debounce = <T extends (...args: any[]) => any>(
  func: T,
  wait: number
): ((...args: Parameters<T>) => void) => {
  let timeout: NodeJS.Timeout | null = null;

  return (...args: Parameters<T>) => {
    if (timeout) clearTimeout(timeout);
    timeout = setTimeout(() => {
      func(...args);
    }, wait);
  };
};

/**
 * Throttle function calls
 */
export const throttle = <T extends (...args: any[]) => any>(
  func: T,
  limit: number
): ((...args: Parameters<T>) => void) => {
  let inThrottle: boolean = false;

  return (...args: Parameters<T>) => {
    if (!inThrottle) {
      func(...args);
      inThrottle = true;
      setTimeout(() => {
        inThrottle = false;
      }, limit);
    }
  };
};

/**
 * Request animation frame wrapper
 */
export const requestAnimationFrameAsync = (callback: () => void): void => {
  requestAnimationFrame(() => {
    callback();
  });
};

/**
 * Performance timing utilities
 */
export class PerformanceTimer {
  private startTime: number = 0;
  private marks: Map<string, number> = new Map();

  start(): void {
    this.startTime = Date.now();
  }

  mark(name: string): void {
    this.marks.set(name, Date.now() - this.startTime);
  }

  getMark(name: string): number | undefined {
    return this.marks.get(name);
  }

  getTotal(): number {
    return Date.now() - this.startTime;
  }

  log(prefix: string = 'Performance'): void {
    console.log(`[${prefix}] Total: ${this.getTotal()}ms`);
    this.marks.forEach((time, name) => {
      console.log(`[${prefix}] ${name}: ${time}ms`);
    });
  }

  reset(): void {
    this.startTime = 0;
    this.marks.clear();
  }
}

/**
 * Memory management utilities
 */
export class MemoryManager {
  /**
   * Check if device is low on memory (iOS only)
   */
  static isLowMemory(): boolean {
    // This would need native module for accurate detection
    // Placeholder for now
    return false;
  }

  /**
   * Clear caches when memory is low
   */
  static clearCaches(): void {
    // Clear image cache, etc.
    // Implementation depends on specific caching solutions used
    console.log('Clearing caches...');
  }

  /**
   * Monitor memory usage
   */
  static monitorMemory(callback: (isLow: boolean) => void): void {
    // This would need native implementation
    // For now, just a placeholder
    if (Platform.OS === 'ios') {
      // iOS has memory warnings we could listen to
      // Would need native module
    }
  }
}

/**
 * Lazy loading utilities
 */
export class LazyLoader {
  private static loadedModules: Set<string> = new Set();

  /**
   * Check if module is loaded
   */
  static isLoaded(moduleName: string): boolean {
    return this.loadedModules.has(moduleName);
  }

  /**
   * Mark module as loaded
   */
  static markLoaded(moduleName: string): void {
    this.loadedModules.add(moduleName);
  }

  /**
   * Preload modules in background
   */
  static preload(modules: string[], callback: () => void): void {
    runAfterInteractions(() => {
      modules.forEach((module) => {
        this.markLoaded(module);
      });
      callback();
    });
  }
}

/**
 * FPS monitoring (for development)
 */
export class FPSMonitor {
  private frameCount: number = 0;
  private lastTime: number = 0;
  private fps: number = 60;

  start(): void {
    this.lastTime = Date.now();
    this.tick();
  }

  private tick = (): void => {
    requestAnimationFrame(() => {
      this.frameCount++;
      const currentTime = Date.now();
      const elapsed = currentTime - this.lastTime;

      if (elapsed >= 1000) {
        this.fps = Math.round((this.frameCount * 1000) / elapsed);
        this.frameCount = 0;
        this.lastTime = currentTime;
      }

      this.tick();
    });
  };

  getFPS(): number {
    return this.fps;
  }
}

/**
 * Batch updates for better performance
 */
export class BatchUpdater {
  private updates: (() => void)[] = [];
  private timeout: NodeJS.Timeout | null = null;

  add(update: () => void): void {
    this.updates.push(update);
    this.scheduleFlush();
  }

  private scheduleFlush(): void {
    if (this.timeout) return;

    this.timeout = setTimeout(() => {
      this.flush();
    }, 16); // ~60fps
  }

  private flush(): void {
    const updates = this.updates.slice();
    this.updates = [];
    this.timeout = null;

    runAfterInteractions(() => {
      updates.forEach((update) => update());
    });
  }
}

/**
 * Image optimization utilities
 */
export class ImageOptimizer {
  /**
   * Get optimized image size based on screen dimensions
   */
  static getOptimizedSize(
    originalWidth: number,
    originalHeight: number,
    maxWidth: number,
    maxHeight: number
  ): { width: number; height: number } {
    const aspectRatio = originalWidth / originalHeight;

    let width = originalWidth;
    let height = originalHeight;

    if (width > maxWidth) {
      width = maxWidth;
      height = width / aspectRatio;
    }

    if (height > maxHeight) {
      height = maxHeight;
      width = height * aspectRatio;
    }

    return {
      width: Math.round(width),
      height: Math.round(height),
    };
  }

  /**
   * Generate thumbnail size
   */
  static getThumbnailSize(
    width: number,
    height: number
  ): { width: number; height: number } {
    return this.getOptimizedSize(width, height, 200, 200);
  }
}

/**
 * Network performance utilities
 */
export class NetworkOptimizer {
  /**
   * Check if connection is slow
   */
  static isSlowConnection(): boolean {
    // This would need native module or web API
    // Placeholder for now
    return false;
  }

  /**
   * Adjust quality based on connection speed
   */
  static getQualityLevel(): 'high' | 'medium' | 'low' {
    if (this.isSlowConnection()) {
      return 'low';
    }
    return 'high';
  }
}
