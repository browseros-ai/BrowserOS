import React, { useState, useEffect, useCallback } from 'react';
import { motion } from 'framer-motion';
import { Mic, Hand, Trash2, Play, Save } from 'lucide-react';

interface GestureMacro {
  id: string;
  name: string;
  type: 'gesture' | 'voice';
  pattern?: string; // For gestures: e.g., "L-shape"
  voicePrint?: string; // For voice: audio fingerprint or keyword
  action: string;
  workflowId?: string;
  createdAt: number;
}

interface GestureMacroRecorderProps {
  onSave?: (macro: GestureMacro) => void;
  onCancel?: () => void;
}

export function GestureMacroRecorder({ onSave, onCancel }: GestureMacroRecorderProps) {
  const [isRecording, setIsRecording] = useState(false);
  const [recordingType, setRecordingType] = useState<'gesture' | 'voice' | null>(null);
  const [recordedPattern, setRecordedPattern] = useState<string>('');
  const [macroName, setMacroName] = useState('');
  const [actionDescription, setActionDescription] = useState('');

  // Canvas for gesture recording
  const canvasRef = React.useRef<HTMLCanvasElement>(null);
  const [isDrawing, setIsDrawing] = useState(false);
  const [points, setPoints] = useState<{x: number, y: number}[]>([]);

  const startGestureRecording = useCallback(() => {
    setRecordingType('gesture');
    setIsRecording(true);
    setPoints([]);
    
    const canvas = canvasRef.current;
    if (canvas) {
      const ctx = canvas.getContext('2d');
      if (ctx) {
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        ctx.strokeStyle = '#3b82f6';
        ctx.lineWidth = 3;
        ctx.lineCap = 'round';
        ctx.lineJoin = 'round';
      }
    }
  }, []);

  const startVoiceRecording = useCallback(async () => {
    setRecordingType('voice');
    setIsRecording(true);

    if (!('webkitSpeechRecognition' in window) && !('SpeechRecognition' in window)) {
      alert('您的瀏覽器不支援語音識別，請使用 Chrome 或 Edge');
      setIsRecording(false);
      return;
    }

    const SpeechRecognition = (window as any).SpeechRecognition || (window as any).webkitSpeechRecognition;
    const recognition = new SpeechRecognition();
    
    recognition.continuous = false;
    recognition.interimResults = true;
    recognition.lang = 'zh-HK';

    recognition.onstart = () => {
      console.log('🎤 開始錄音...');
    };

    recognition.onresult = (event: any) => {
      const transcript = Array.from(event.results)
        .map((result: any) => result[0])
        .map((result) => result.transcript)
        .join('');
      
      setRecordedPattern(transcript);
      setMacroName(transcript.slice(0, 30));
    };

    recognition.onend = () => {
      setIsRecording(false);
      console.log('🎤 錄音結束');
    };

    recognition.onerror = (event: any) => {
      console.error('語音識別錯誤:', event.error);
      setIsRecording(false);
    };

    try {
      recognition.start();
    } catch (error) {
      console.error('無法啟動語音識別:', error);
    }
  }, []);

  const stopRecording = useCallback(() => {
    setIsRecording(false);
    setRecordingType(null);
  }, []);

  // Canvas mouse/touch handlers
  const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!isRecording || recordingType !== 'gesture') return;
    
    setIsDrawing(true);
    const canvas = canvasRef.current;
    if (!canvas) return;
    
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    
    setPoints([{ x, y }]);
    
    const ctx = canvas.getContext('2d');
    if (ctx) {
      ctx.beginPath();
      ctx.moveTo(x, y);
    }
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (!isDrawing || !isRecording || recordingType !== 'gesture') return;
    
    const canvas = canvasRef.current;
    if (!canvas) return;
    
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    
    setPoints(prev => [...prev, { x, y }]);
    
    const ctx = canvas.getContext('2d');
    if (ctx) {
      ctx.lineTo(x, y);
      ctx.stroke();
    }
  };

  const handleMouseUp = () => {
    if (!isDrawing) return;
    setIsDrawing(false);
    
    // Analyze gesture pattern
    analyzeGesturePattern(points);
  };

  const analyzeGesturePattern = (pts: {x: number, y: number}[]) => {
    if (pts.length < 10) return;
    
    // Simple pattern recognition based on direction changes
    let directionChanges = 0;
    for (let i = 1; i < pts.length - 1; i++) {
      const prevDx = pts[i].x - pts[i-1].x;
      const prevDy = pts[i].y - pts[i-1].y;
      const nextDx = pts[i+1].x - pts[i].x;
      const nextDy = pts[i+1].y - pts[i].y;
      
      // Detect significant direction change
      if ((prevDx > 0 && nextDx < 0) || (prevDx < 0 && nextDx > 0) ||
          (prevDy > 0 && nextDy < 0) || (prevDy < 0 && nextDy > 0)) {
        directionChanges++;
      }
    }
    
    // Classify gesture
    let pattern = 'unknown';
    if (directionChanges === 1) {
      pattern = 'L-shape';
    } else if (directionChanges === 2) {
      pattern = 'Z-shape';
    } else if (directionChanges >= 3) {
      pattern = 'complex';
    } else {
      pattern = 'line';
    }
    
    setRecordedPattern(pattern);
    setMacroName(`${pattern} 手勢`);
  };

  const clearCanvas = useCallback(() => {
    const canvas = canvasRef.current;
    if (canvas) {
      const ctx = canvas.getContext('2d');
      if (ctx) {
        ctx.clearRect(0, 0, canvas.width, canvas.height);
      }
    }
    setPoints([]);
    setRecordedPattern('');
    setMacroName('');
  }, []);

  const handleSave = () => {
    if (!macroName || !actionDescription) {
      alert('請填寫快捷鍵名稱和動作描述');
      return;
    }

    const macro: GestureMacro = {
      id: `macro_${Date.now()}`,
      name: macroName,
      type: recordingType || 'gesture',
      pattern: recordingType === 'gesture' ? recordedPattern : undefined,
      voicePrint: recordingType === 'voice' ? recordedPattern : undefined,
      action: actionDescription,
      createdAt: Date.now(),
    };

    onSave?.(macro);
  };

  return (
    <motion.div
      initial={{ opacity: 0, scale: 0.95 }}
      animate={{ opacity: 1, scale: 1 }}
      className="bg-white dark:bg-gray-800 rounded-2xl shadow-xl p-6 max-w-lg mx-auto"
    >
      <h3 className="text-xl font-bold text-gray-900 dark:text-white mb-4">
        🎯 錄製個人快捷鍵
      </h3>

      {/* Recording Type Selection */}
      {!recordingType && (
        <div className="grid grid-cols-2 gap-4 mb-6">
          <motion.button
            whileHover={{ scale: 1.05 }}
            whileTap={{ scale: 0.95 }}
            onClick={startGestureRecording}
            disabled={isRecording}
            className="flex flex-col items-center gap-3 p-6 bg-gradient-to-br from-blue-50 to-blue-100 dark:from-blue-900/30 dark:to-blue-800/30 rounded-xl border-2 border-blue-200 dark:border-blue-700 hover:border-blue-400 transition-colors"
          >
            <Hand className="w-8 h-8 text-blue-600 dark:text-blue-400" />
            <span className="font-medium text-blue-900 dark:text-blue-100">
              手勢快捷鍵
            </span>
            <span className="text-xs text-blue-600 dark:text-blue-400">
              繪製圖形觸發
            </span>
          </motion.button>

          <motion.button
            whileHover={{ scale: 1.05 }}
            whileTap={{ scale: 0.95 }}
            onClick={startVoiceRecording}
            disabled={isRecording}
            className="flex flex-col items-center gap-3 p-6 bg-gradient-to-br from-purple-50 to-purple-100 dark:from-purple-900/30 dark:to-purple-800/30 rounded-xl border-2 border-purple-200 dark:border-purple-700 hover:border-purple-400 transition-colors"
          >
            <Mic className="w-8 h-8 text-purple-600 dark:text-purple-400" />
            <span className="font-medium text-purple-900 dark:text-purple-100">
              聲音快捷鍵
            </span>
            <span className="text-xs text-purple-600 dark:text-purple-400">
              錄製聲音觸發
            </span>
          </motion.button>
        </div>
      )}

      {/* Gesture Recording Canvas */}
      {recordingType === 'gesture' && (
        <div className="mb-6">
          <div className="flex items-center justify-between mb-2">
            <p className="text-sm text-gray-600 dark:text-gray-400">
              {isRecording ? '✏️ 在下方繪製您的手勢...' : '✅ 手勢已記錄'}
            </p>
            <button
              onClick={clearCanvas}
              className="p-2 hover:bg-gray-100 dark:hover:bg-gray-700 rounded-lg"
            >
              <Trash2 className="w-4 h-4 text-gray-500" />
            </button>
          </div>
          
          <canvas
            ref={canvasRef}
            width={400}
            height={250}
            className="w-full border-2 border-dashed border-gray-300 dark:border-gray-600 rounded-xl bg-gray-50 dark:bg-gray-900 cursor-crosshair touch-none"
            onMouseDown={handleMouseDown}
            onMouseMove={handleMouseMove}
            onMouseUp={handleMouseUp}
            onMouseLeave={handleMouseUp}
          />
          
          {recordedPattern && (
            <div className="mt-2 px-3 py-2 bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-400 rounded-lg text-sm">
              ✅ 識別為：<strong>{recordedPattern}</strong>
            </div>
          )}
        </div>
      )}

      {/* Voice Recording Status */}
      {recordingType === 'voice' && (
        <div className="mb-6 p-6 bg-gradient-to-br from-purple-50 to-purple-100 dark:from-purple-900/30 dark:to-purple-800/30 rounded-xl text-center">
          {isRecording ? (
            <motion.div
              animate={{ scale: [1, 1.2, 1] }}
              transition={{ repeat: Infinity, duration: 1.5 }}
              className="flex flex-col items-center gap-3"
            >
              <Mic className="w-12 h-12 text-purple-600 dark:text-purple-400" />
              <p className="text-purple-900 dark:text-purple-100 font-medium">
                🎤 正在錄音...
              </p>
              <p className="text-purple-600 dark:text-purple-400 text-sm">
                請說出您的快捷指令（例如：輕咳兩聲）
              </p>
            </motion.div>
          ) : (
            <div className="flex flex-col items-center gap-3">
              <span className="text-4xl">✅</span>
              <p className="text-purple-900 dark:text-purple-100 font-medium">
                已錄製：{recordedPattern || '無聲'}
              </p>
            </div>
          )}
        </div>
      )}

      {/* Macro Configuration */}
      {(recordedPattern || !isRecording) && recordingType && (
        <div className="space-y-4">
          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              快捷鍵名稱
            </label>
            <input
              type="text"
              value={macroName}
              onChange={(e) => setMacroName(e.target.value)}
              placeholder="例如：L 形手勢、輕咳兩聲"
              className="w-full px-4 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500"
            />
          </div>

          <div>
            <label className="block text-sm font-medium text-gray-700 dark:text-gray-300 mb-1">
              執行動作
            </label>
            <textarea
              value={actionDescription}
              onChange={(e) => setActionDescription(e.target.value)}
              placeholder="例如：開啟靜音模式、執行自動化工作流..."
              rows={3}
              className="w-full px-4 py-2 border border-gray-300 dark:border-gray-600 rounded-lg bg-white dark:bg-gray-700 text-gray-900 dark:text-white focus:ring-2 focus:ring-blue-500"
            />
          </div>

          {/* Action Buttons */}
          <div className="flex gap-3 pt-4">
            <motion.button
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={handleSave}
              className="flex-1 flex items-center justify-center gap-2 px-4 py-3 bg-gradient-to-r from-blue-500 to-blue-600 text-white rounded-lg font-medium shadow-lg hover:shadow-xl transition-shadow"
            >
              <Save className="w-5 h-5" />
              儲存快捷鍵
            </motion.button>

            <motion.button
              whileHover={{ scale: 1.02 }}
              whileTap={{ scale: 0.98 }}
              onClick={onCancel}
              className="px-4 py-3 bg-gray-100 dark:bg-gray-700 text-gray-700 dark:text-gray-300 rounded-lg font-medium hover:bg-gray-200 dark:hover:bg-gray-600 transition-colors"
            >
              取消
            </motion.button>
          </div>
        </div>
      )}

      {/* Instructions */}
      <div className="mt-6 p-4 bg-blue-50 dark:bg-blue-900/20 rounded-lg">
        <p className="text-xs text-blue-700 dark:text-blue-400">
          💡 <strong>提示：</strong>您可以錄製簡單的手勢（如 L 形、Z 形）或獨特的聲音（如口哨聲、擬聲詞）作為快捷鍵，用於觸發複雜的工作流程。
        </p>
      </div>
    </motion.div>
  );
}

export default GestureMacroRecorder;
