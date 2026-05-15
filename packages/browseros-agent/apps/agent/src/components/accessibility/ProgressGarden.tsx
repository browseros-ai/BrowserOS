import React, { useEffect, useState } from 'react';
import { motion } from 'framer-motion';

interface GardenItem {
  id: string;
  type: 'plant' | 'star' | 'crystal';
  stage: number;
  completedAt: Date;
}

export const ProgressGarden: React.FC = () => {
  const [garden, setGarden] = useState<GardenItem[]>([]);

  useEffect(() => {
    // Load garden state
    const saved = localStorage.getItem('progress_garden');
    if (saved) {
      setGarden(JSON.parse(saved));
    }
  }, []);

  const addGrowth = (taskName: string) => {
    const newItem: GardenItem = {
      id: Date.now().toString(),
      type: ['plant', 'star', 'crystal'][Math.floor(Math.random() * 3)] as any,
      stage: 1,
      completedAt: new Date(),
    };
    const updated = [...garden, newItem];
    setGarden(updated);
    localStorage.setItem('progress_garden', JSON.stringify(updated));
  };

  return (
    <div className="p-4 bg-gradient-to-b from-sky-100 to-green-50 dark:from-slate-900 dark:to-slate-800 rounded-xl min-h-[200px]">
      <h3 className="text-sm font-medium mb-4 text-muted-foreground">🌱 成長花園</h3>
      <div className="flex flex-wrap gap-4">
        {garden.length === 0 ? (
          <p className="text-sm text-muted-foreground">完成任務來種植您的第一株植物！</p>
        ) : (
          garden.map((item, i) => (
            <motion.div
              key={item.id}
              initial={{ scale: 0, opacity: 0 }}
              animate={{ scale: 1, opacity: 1 }}
              transition={{ delay: i * 0.1, type: 'spring' }}
              className="w-16 h-16 flex items-center justify-center text-4xl"
            >
              {item.type === 'plant' && '🌿'}
              {item.type === 'star' && '⭐'}
              {item.type === 'crystal' && '💎'}
            </motion.div>
          ))
        )}
      </div>
    </div>
  );
};
