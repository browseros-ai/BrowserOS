import React, { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Eye, Type, Moon, Sun, Monitor, Maximize2, X } from 'lucide-react';

interface GlobalOverlayProps {
  isVisible: boolean;
  onClose: () => void;
}

type OverlayMode = 'focus' | 'visual' | 'normal';

export function GlobalOverlay({ isVisible, onClose }: GlobalOverlayProps) {
  const [mode, setMode] = useState<OverlayMode>('normal');
  const [isListening, setIsListening] = useState(false);

  // Wake word detection simulation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      // Ctrl+Shift+A to toggle overlay
      if (e.ctrlKey && e.shiftKey && e.key === 'A') {
        e.preventDefault();
        onClose();
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  // Voice wake word detection
  useEffect(() => {
    if (!('webkitSpeechRecognition' in window) && !('SpeechRecognition' in window)) {
      return;
    }

    const SpeechRecognition = (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;
    const recognition = new SpeechRecognition();
    
    recognition.continuous = true;
    recognition.interimResults = true;
    recognition.lang = 'zh-HK';

    recognition.onresult = (event: any) => {
      const transcript = event.results[event.results.length - 1][0].transcript.toLowerCase();
      
      // Wake word detection
      if (transcript.includes('hey browseros') || transcript.includes('browseros')) {
        setIsListening(true);
        setTimeout(() => setIsListening(false), 2000);
      }
    };

    try {
      recognition.start();
    } catch (error) {
      console.log('Speech recognition not available:', error);
    }

    return () => {
      try {
        recognition.stop();
      } catch (error) {
        // Ignore stop errors
      }
    };
  }, []);

  const applyFocusMode = useCallback(() => {
    // Remove ads, sidebars, popups
    const selectorsToRemove = [
      'iframe[src*="ad"]',
      '.advertisement',
      '.sidebar',
      '.popup',
      '[class*="ad-"]',
      '[id*="ad-"]',
      '.cookie-banner',
      '.newsletter-signup',
    ];

    selectorsToRemove.forEach(selector => {
      document.querySelectorAll(selector).forEach(el => {
        (el as HTMLElement).style.display = 'none';
      });
    });

    // Apply dyslexia-friendly styles
    document.body.style.fontFamily = 'OpenDyslexic, Arial, sans-serif';
    document.body.style.lineHeight = '1.8';
    document.body.style.letterSpacing = '0.05em';
    document.body.style.maxWidth = '800px';
    document.body.style.margin = '0 auto';
    document.body.style.padding = '20px';

    setMode('focus');
  }, []);

  const applyVisualMode = useCallback(() => {
    // Generate AI summary icons for text content
    const paragraphs = document.querySelectorAll('p');
    paragraphs.forEach(p => {
      const text = p.textContent?.slice(0, 100) || '';
      if (text.length > 50 && !p.querySelector('.ai-summary-icon')) {
        const icon = document.createElement('span');
        icon.className = 'ai-summary-icon';
        icon.innerHTML = ' 📊';
        icon.style.cssText = 'cursor: pointer; margin-left: 8px; font-size: 16px;';
        icon.title = 'AI 摘要：點擊查看圖示說明';
        p.insertBefore(icon, p.firstChild);
      }
    });

    setMode('visual');
  }, []);

  const resetMode = useCallback(() => {
    // Reset all styles
    document.body.style.fontFamily = '';
    document.body.style.lineHeight = '';
    document.body.style.letterSpacing = '';
    document.body.style.maxWidth = '';
    document.body.style.margin = '';
    document.body.style.padding = '';

    // Show hidden elements
    document.querySelectorAll('[style*="display: none"]').forEach(el => {
      (el as HTMLElement).style.display = '';
    });

    // Remove AI icons
    document.querySelectorAll('.ai-summary-icon').forEach(el => {
      el.remove();
    });

    setMode('normal');
  }, []);

  if (!isVisible) return null;

  return (
    <AnimatePresence>
      <motion.div
        initial={{ opacity: 0, y: -20 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: -20 }}
        className="fixed top-4 left-1/2 transform -translate-x-1/2 z-[9999] flex items-center gap-2 px-6 py-3 bg-white/95 dark:bg-gray-900/95 backdrop-blur-lg rounded-2xl shadow-2xl border border-gray-200 dark:border-gray-700"
      >
        {/* Voice Status Indicator */}
        {isListening && (
          <motion.div
            initial={{ scale: 0 }}
            animate={{ scale: 1 }}
            className="absolute -top-12 left-1/2 transform -translate-x-1/2 px-3 py-1 bg-blue-500 text-white text-xs rounded-full"
          >
            👂 聆聽中...
          </motion.div>
        )}

        {/* Mode Buttons */}
        <motion.button
          whileHover={{ scale: 1.1 }}
          whileTap={{ scale: 0.9 }}
          onClick={applyFocusMode}
          className={`p-2 rounded-lg transition-colors ${
            mode === 'focus' 
              ? 'bg-blue-100 dark:bg-blue-900 text-blue-600 dark:text-blue-400' 
              : 'hover:bg-gray-100 dark:hover:bg-gray-800'
          }`}
          title="專注閱讀模式 (Ctrl+Shift+A)"
        >
          <Eye className="w-5 h-5" />
        </motion.button>

        <motion.button
          whileHover={{ scale: 1.1 }}
          whileTap={{ scale: 0.9 }}
          onClick={applyVisualMode}
          className={`p-2 rounded-lg transition-colors ${
            mode === 'visual' 
              ? 'bg-purple-100 dark:bg-purple-900 text-purple-600 dark:text-purple-400' 
              : 'hover:bg-gray-100 dark:hover:bg-gray-800'
          }`}
          title="圖文並茂模式"
        >
          <Type className="w-5 h-5" />
        </motion.button>

        <motion.button
          whileHover={{ scale: 1.1 }}
          whileTap={{ scale: 0.9 }}
          onClick={resetMode}
          className={`p-2 rounded-lg transition-colors ${
            mode === 'normal' 
              ? 'bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400' 
              : 'hover:bg-gray-100 dark:hover:bg-gray-800'
          }`}
          title="恢復正常模式"
        >
          <Monitor className="w-5 h-5" />
        </motion.button>

        {/* Divider */}
        <div className="w-px h-6 bg-gray-300 dark:bg-gray-700 mx-1" />

        {/* Quick Actions */}
        <motion.button
          whileHover={{ scale: 1.1 }}
          whileTap={{ scale: 0.9 }}
          className="p-2 hover:bg-gray-100 dark:hover:bg-gray-800 rounded-lg"
          title="語音筆記"
        >
          <Maximize2 className="w-5 h-5" />
        </motion.button>

        {/* Close Button */}
        <motion.button
          whileHover={{ scale: 1.1 }}
          whileTap={{ scale: 0.9 }}
          onClick={onClose}
          className="p-2 hover:bg-red-100 dark:hover:bg-red-900 rounded-lg text-red-500"
          title="關閉 (Ctrl+Shift+A)"
        >
          <X className="w-5 h-5" />
        </motion.button>

        {/* Mode Indicator */}
        <div className="ml-2 px-3 py-1 bg-gray-100 dark:bg-gray-800 rounded-full text-xs font-medium text-gray-600 dark:text-gray-400">
          {mode === 'focus' && '📖 專注模式'}
          {mode === 'visual' && '🎨 圖文模式'}
          {mode === 'normal' && '💻 正常模式'}
        </div>
      </motion.div>
    </AnimatePresence>
  );
}

export default GlobalOverlay;
