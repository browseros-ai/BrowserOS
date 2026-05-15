import React, { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence, Reorder } from 'framer-motion';
import { useAccessibility } from '../../hooks/useAccessibility';
import { VoiceCommandOrb } from './VoiceCommandOrb';
import { StrengthMatcher } from './StrengthMatcher';

interface DashboardItem {
  id: string;
  type: 'workflow' | 'tool' | 'shortcut';
  icon: string;
  label: string;
  action: () => void;
  frequency: number;
  lastUsed?: Date;
}

export const SensoryDashboard: React.FC = () => {
  const { currentProfile, adjustSettings } = useAccessibility();
  const [items, setItems] = useState<DashboardItem[]>([]);
  const [isOrbActive, setIsOrbActive] = useState(false);
  const [showStrengths, setShowStrengths] = useState(false);

  // Auto-sort items by frequency and recency
  const sortedItems = [...items].sort((a, b) => {
    const scoreA = a.frequency * (a.lastUsed ? new Date(a.lastUsed).getTime() : 0);
    const scoreB = b.frequency * (b.lastUsed ? new Date(b.lastUsed).getTime() : 0);
    return scoreB - scoreA;
  });

  // Auto-hide unused UI based on profile
  const shouldHideUnused = currentProfile?.settings.visual?.hideUnusedUI;

  // Handle voice command activation
  const handleVoiceActivate = useCallback(() => {
    setIsOrbActive(true);
  }, []);

  // Handle item reordering
  const handleReorder = useCallback((newItems: DashboardItem[]) => {
    setItems(newItems);
    // Save preference to backend
  }, []);

  return (
    <div className={`
      relative min-h-screen transition-all duration-300
      ${currentProfile?.settings.visual?.layout === 'single-column' ? 'max-w-2xl mx-auto' : ''}
      ${currentProfile?.settings.visual?.colorScheme === 'monochrome-focus' ? 'grayscale-[0.8]' : ''}
    `}>
      {/* Voice Command Orb - Always accessible */}
      <VoiceCommandOrb 
        isActive={isOrbActive}
        onActivate={handleVoiceActivate}
        onDeactivate={() => setIsOrbActive(false)}
      />

      {/* Main Dashboard Area */}
      <main className="p-4 md:p-8">
        <motion.div
          initial={{ opacity: 0, y: 20 }}
          animate={{ opacity: 1, y: 0 }}
          className="mb-8"
        >
          <h1 className={`
            text-2xl md:text-3xl font-light mb-2
            ${currentProfile?.id === 'hsp-calm' ? 'text-muted-foreground' : 'text-foreground'}
          `}>
            {currentProfile?.icon} 歡迎回來
          </h1>
          <p className="text-sm text-muted-foreground">
            已自動套用 {currentProfile?.name}
          </p>
        </motion.div>

        {/* Draggable Icon Grid - De-gridified */}
        <Reorder.Group
          axis="y"
          values={sortedItems}
          onReorder={handleReorder}
          className="space-y-3"
        >
          <AnimatePresence>
            {sortedItems.map((item) => (
              <Reorder.Item
                key={item.id}
                value={item}
                whileDrag={{ scale: 1.05 }}
                className="relative"
              >
                <motion.button
                  onClick={item.action}
                  whileHover={{ scale: 1.02 }}
                  whileTap={{ scale: 0.98 }}
                  className={`
                    w-full p-4 rounded-xl
                    flex items-center gap-4
                    bg-card hover:bg-accent/10
                    border border-border/50
                    transition-all duration-200
                    ${shouldHideUnused && item.frequency === 0 ? 'opacity-40 hover:opacity-100' : ''}
                  `}
                >
                  <span className="text-3xl">{item.icon}</span>
                  <div className="flex-1 text-left">
                    <h3 className="font-medium">{item.label}</h3>
                    {item.lastUsed && (
                      <p className="text-xs text-muted-foreground">
                        上次使用：{new Date(item.lastUsed).toLocaleDateString('zh-HK')}
                      </p>
                    )}
                  </div>
                  <motion.div
                    className="w-2 h-2 rounded-full bg-primary"
                    animate={{
                      scale: [1, 1.2, 1],
                    }}
                    transition={{
                      duration: 2,
                      repeat: Infinity,
                      delay: Math.random() * 2,
                    }}
                  />
                </motion.button>
              </Reorder.Item>
            ))}
          </AnimatePresence>
        </Reorder.Group>

        {/* Empty State with Gentle Prompt */}
        {sortedItems.length === 0 && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            className="text-center py-16"
          >
            <p className="text-muted-foreground mb-4">
              開始使用後，您的常用工具會自動出現在這裡
            </p>
            <button
              onClick={() => setShowStrengths(true)}
              className="text-primary hover:underline text-sm"
            >
              探索推薦工具 →
            </button>
          </motion.div>
        )}
      </main>

      {/* AI Strength Matcher Sidebar */}
      <AnimatePresence>
        {showStrengths && (
          <StrengthMatcher
            onClose={() => setShowStrengths(false)}
            onApplyTool={(tool) => {
              // Add tool to dashboard
              setShowStrengths(false);
            }}
          />
        )}
      </AnimatePresence>
    </div>
  );
};
