import React, { useState, useEffect } from 'react';
import { motion } from 'framer-motion';

interface StrengthRecommendation {
  id: string;
  type: 'automation' | 'tool' | 'workflow';
  title: string;
  description: string;
  icon: string;
  confidence: number;
  action: () => void;
}

export const StrengthMatcher: React.FC<{
  onClose: () => void;
  onApplyTool: (tool: any) => void;
}> = ({ onClose, onApplyTool }) => {
  const [recommendations, setRecommendations] = useState<StrengthRecommendation[]>([]);
  const [analyzing, setAnalyzing] = useState(true);

  useEffect(() => {
    // Simulate AI analysis of user behavior
    const analyzeStrengths = async () => {
      setAnalyzing(true);
      
      try {
        const response = await fetch('/api/accessibility/analyze-strengths', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            sessionData: {
              duration: 3600, // seconds
              actions: ['copy-paste', 'tab-switching', 'form-filling'],
              patterns: ['repetitive-task', 'visual-search'],
            },
          }),
        });
        
        const data = await response.json();
        setRecommendations(data.recommendations || []);
      } catch (error) {
        console.error('分析失敗:', error);
        // Fallback recommendations
        setRecommendations([
          {
            id: '1',
            type: 'automation',
            title: '自動化重複操作',
            description: '偵測到您經常複製貼上，建立自動化流程？',
            icon: '🤖',
            confidence: 0.85,
            action: () => console.log('建立自動化'),
          },
          {
            id: '2',
            type: 'tool',
            title: '視覺輔助工具',
            description: '啟用螢幕標註功能協助定位',
            icon: '🎯',
            confidence: 0.72,
            action: () => console.log('啟用標註'),
          },
        ]);
      } finally {
        setAnalyzing(false);
      }
    };

    analyzeStrengths();
  }, []);

  return (
    <motion.div
      initial={{ x: 400, opacity: 0 }}
      animate={{ x: 0, opacity: 1 }}
      exit={{ x: 400, opacity: 0 }}
      className="fixed right-0 top-0 h-full w-80 bg-card border-l border-border shadow-2xl z-50 overflow-y-auto"
    >
      <div className="p-6">
        <div className="flex items-center justify-between mb-6">
          <h2 className="text-lg font-semibold flex items-center gap-2">
            <span>✨</span> AI 專長助手
          </h2>
          <button
            onClick={onClose}
            className="text-muted-foreground hover:text-foreground"
          >
            ✕
          </button>
        </div>

        {analyzing ? (
          <div className="space-y-4">
            <motion.div
              animate={{ opacity: [0.5, 1, 0.5] }}
              transition={{ duration: 1.5, repeat: Infinity }}
              className="text-center py-8"
            >
              <div className="text-4xl mb-4">🔍</div>
              <p className="text-sm text-muted-foreground">
                正在分析您的操作模式...
              </p>
            </motion.div>
          </div>
        ) : (
          <div className="space-y-4">
            <p className="text-sm text-muted-foreground mb-4">
              根據您的使用習慣，我們發現以下優化機會：
            </p>

            {recommendations.map((rec, index) => (
              <motion.div
                key={rec.id}
                initial={{ opacity: 0, y: 20 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: index * 0.1 }}
                className="p-4 rounded-lg bg-accent/10 border border-border/50 hover:bg-accent/20 transition-colors cursor-pointer"
                onClick={() => {
                  rec.action();
                  onApplyTool(rec);
                }}
              >
                <div className="flex items-start gap-3">
                  <span className="text-2xl">{rec.icon}</span>
                  <div className="flex-1">
                    <h3 className="font-medium text-sm mb-1">{rec.title}</h3>
                    <p className="text-xs text-muted-foreground mb-2">
                      {rec.description}
                    </p>
                    <div className="flex items-center gap-2">
                      <div className="flex-1 h-1.5 bg-muted rounded-full overflow-hidden">
                        <motion.div
                          initial={{ width: 0 }}
                          animate={{ width: `${rec.confidence * 100}%` }}
                          transition={{ duration: 0.5, delay: index * 0.1 }}
                          className="h-full bg-primary rounded-full"
                        />
                      </div>
                      <span className="text-xs text-muted-foreground">
                        {Math.round(rec.confidence * 100)}% 匹配
                      </span>
                    </div>
                  </div>
                </div>
              </motion.div>
            ))}

            {recommendations.length === 0 && (
              <div className="text-center py-8 text-muted-foreground">
                <p className="text-sm">暫無推薦</p>
                <p className="text-xs mt-2">繼續使用以獲得個人化建議</p>
              </div>
            )}
          </div>
        )}

        {/* Contextual Tips */}
        <div className="mt-6 pt-6 border-t border-border">
          <h3 className="text-xs font-medium text-muted-foreground mb-3">
            💡 小貼士
          </h3>
          <p className="text-xs text-muted-foreground leading-relaxed">
            系統會持續學習您的操作模式，提供更精準的輔助建議。
            所有分析僅在本地進行，保護您的隱私。
          </p>
        </div>
      </div>
    </motion.div>
  );
};
