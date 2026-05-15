import React, { useState, useEffect, useRef, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { useAccessibility } from '../../hooks/useAccessibility';

interface OrbState {
  status: 'idle' | 'listening' | 'processing' | 'success' | 'error';
  waveform: number[];
  recognizedText: string;
}

export const VoiceCommandOrb: React.FC<{
  isActive: boolean;
  onActivate: () => void;
  onDeactivate: () => void;
}> = ({ isActive, onActivate, onDeactivate }) => {
  const { currentProfile } = useAccessibility();
  const [orbState, setOrbState] = useState<OrbState>({
    status: 'idle',
    waveform: [],
    recognizedText: '',
  });
  
  const recognitionRef = useRef<any>(null);
  const animationFrameRef = useRef<number>();

  // Generate smooth waveform for visualization
  const generateWaveform = useCallback(() => {
    if (orbState.status === 'listening') {
      const newWaveform = Array.from({ length: 20 }, () => 
        Math.random() * 0.8 + 0.2
      );
      setOrbState(prev => ({ ...prev, waveform: newWaveform }));
      animationFrameRef.current = requestAnimationFrame(generateWaveform);
    } else {
      setOrbState(prev => ({ ...prev, waveform: Array(20).fill(0.3) }));
    }
  }, [orbState.status]);

  useEffect(() => {
    if (isActive && orbState.status === 'listening') {
      generateWaveform();
    }
    return () => {
      if (animationFrameRef.current) {
        cancelAnimationFrame(animationFrameRef.current);
      }
    };
  }, [isActive, orbState.status, generateWaveform]);

  // Initialize speech recognition
  useEffect(() => {
    if ('webkitSpeechRecognition' in window || 'SpeechRecognition' in window) {
      const SpeechRecognition = (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;
      recognitionRef.current = new SpeechRecognition();
      recognitionRef.current.continuous = true;
      recognitionRef.current.interimResults = true;
      recognitionRef.current.lang = 'zh-HK'; // Default to Cantonese HK

      recognitionRef.current.onresult = (event: any) => {
        const transcript = Array.from(event.results)
          .map((result: any) => result[0])
          .map((result) => result.transcript)
          .join('');
        
        setOrbState(prev => ({ ...prev, recognizedText: transcript }));

        // Auto-detect language mix
        const hasEnglish = /[a-zA-Z]/.test(transcript);
        const hasChinese = /[\u4e00-\u9fa5]/.test(transcript);
        
        if (hasEnglish && hasChinese) {
          // Mixed language detected - handle accordingly
          console.log('🎤 多語言混合指令:', transcript);
        }

        // Check for wake word or command completion
        if (transcript.includes('執行') || transcript.includes('打開') || transcript.includes('建立')) {
          processCommand(transcript);
        }
      };

      recognitionRef.current.onerror = (event: any) => {
        console.error('語音識別錯誤:', event.error);
        setOrbState(prev => ({ ...prev, status: 'error' }));
        setTimeout(() => {
          setOrbState(prev => ({ ...prev, status: 'idle' }));
          onDeactivate();
        }, 2000);
      };

      recognitionRef.current.onend = () => {
        if (isActive && orbState.status === 'listening') {
          recognitionRef.current?.start();
        }
      };
    }

    return () => {
      if (recognitionRef.current) {
        recognitionRef.current.stop();
      }
    };
  }, []);

  // Start/stop listening based on active state
  useEffect(() => {
    if (isActive && recognitionRef.current) {
      try {
        recognitionRef.current.start();
        setOrbState(prev => ({ ...prev, status: 'listening' }));
      } catch (err) {
        console.error('無法啟動語音識別:', err);
      }
    } else if (!isActive && recognitionRef.current) {
      recognitionRef.current.stop();
      setOrbState(prev => ({ ...prev, status: 'idle' }));
    }
  }, [isActive]);

  // Process voice command
  const processCommand = async (command: string) => {
    setOrbState(prev => ({ ...prev, status: 'processing' }));
    
    // Send to backend for processing
    try {
      const response = await fetch('/api/accessibility/voice-command', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ command, profileId: currentProfile?.id }),
      });
      
      const result = await response.json();
      
      if (result.success) {
        setOrbState(prev => ({ ...prev, status: 'success' }));
        setTimeout(() => {
          setOrbState(prev => ({ ...prev, status: 'idle' }));
          onDeactivate();
        }, 1500);
      } else {
        throw new Error(result.message);
      }
    } catch (error) {
      console.error('命令處理失敗:', error);
      setOrbState(prev => ({ ...prev, status: 'error' }));
      setTimeout(() => {
        setOrbState(prev => ({ ...prev, status: 'idle' }));
        onDeactivate();
      }, 2000);
    }
  };

  // Orb color based on state
  const getOrbColor = () => {
    switch (orbState.status) {
      case 'listening': return 'bg-blue-500';
      case 'processing': return 'bg-purple-500';
      case 'success': return 'bg-green-500';
      case 'error': return 'bg-red-500';
      default: return 'bg-primary/30';
    }
  };

  // Glow effect intensity
  const getGlowIntensity = () => {
    switch (orbState.status) {
      case 'listening': return 'shadow-[0_0_30px_rgba(59,130,246,0.6)]';
      case 'processing': return 'shadow-[0_0_40px_rgba(168,85,247,0.7)]';
      case 'success': return 'shadow-[0_0_25px_rgba(34,197,94,0.5)]';
      default: return 'shadow-[0_0_15px_rgba(0,0,0,0.2)]';
    }
  };

  return (
    <div className="fixed bottom-6 right-6 z-50 flex flex-col items-end gap-4">
      {/* Recognized Text Display */}
      <AnimatePresence>
        {orbState.recognizedText && orbState.status === 'listening' && (
          <motion.div
            initial={{ opacity: 0, x: 20 }}
            animate={{ opacity: 1, x: 0 }}
            exit={{ opacity: 0, x: 20 }}
            className="bg-card/90 backdrop-blur-sm px-4 py-2 rounded-lg max-w-xs border border-border"
          >
            <p className="text-sm text-muted-foreground line-clamp-2">
              {orbState.recognizedText}
            </p>
          </motion.div>
        )}
      </AnimatePresence>

      {/* The Orb */}
      <motion.button
        onClick={() => isActive ? onDeactivate() : onActivate()}
        whileHover={{ scale: 1.1 }}
        whileTap={{ scale: 0.95 }}
        className={`
          relative w-14 h-14 rounded-full
          ${getOrbColor()}
          ${getGlowIntensity()}
          transition-all duration-300
          flex items-center justify-center
        `}
      >
        {/* Waveform Visualization */}
        {orbState.status === 'listening' && (
          <div className="absolute inset-0 flex items-center justify-center gap-0.5">
            {orbState.waveform.map((amplitude, i) => (
              <motion.div
                key={i}
                className="w-0.5 bg-white/80 rounded-full"
                animate={{
                  height: `${amplitude * 40}px`,
                }}
                transition={{
                  duration: 0.1,
                  ease: 'easeInOut',
                }}
              />
            ))}
          </div>
        )}

        {/* Status Icon */}
        <span className="relative z-10 text-white text-xl">
          {orbState.status === 'idle' && '🎤'}
          {orbState.status === 'listening' && '👂'}
          {orbState.status === 'processing' && '⚙️'}
          {orbState.status === 'success' && '✓'}
          {orbState.status === 'error' && '!'}
        </span>

        {/* Breathing Animation for Idle State */}
        {orbState.status === 'idle' && (
          <motion.div
            className="absolute inset-0 rounded-full bg-primary/20"
            animate={{
              scale: [1, 1.2, 1],
              opacity: [0.5, 0.8, 0.5],
            }}
            transition={{
              duration: 2,
              repeat: Infinity,
              ease: 'easeInOut',
            }}
          />
        )}
      </motion.button>

      {/* Quick Hint */}
      {orbState.status === 'idle' && !isActive && (
        <motion.p
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          className="text-xs text-muted-foreground"
        >
          點擊或說 "Hey BrowserOS"
        </motion.p>
      )}
    </div>
  );
};
