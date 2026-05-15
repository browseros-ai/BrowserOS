import { Hono } from 'hono';
import { multimodalService } from '../services/accessibility/multimodal-service';

export const accessibilityRouter = new Hono();

// Detect optimal profile based on user signals
accessibilityRouter.post('/detect-profile', async (c) => {
  try {
    const body = await c.req.json();
    const { preferences, behavior } = body;
    
    // Simple heuristic-based recommendation
    let recommendedProfile = 'adhd-focus'; // default
    
    if (preferences?.prefersReducedMotion) {
      recommendedProfile = 'hsp-calm';
    } else if (preferences?.prefersContrast) {
      recommendedProfile = 'visual-impaired';
    }
    
    // Analyze behavior patterns
    if (behavior?.tabSwitches > 20) {
      recommendedProfile = 'adhd-focus';
    }
    
    return c.json({
      success: true,
      recommendedProfile,
      confidence: 0.75,
    });
  } catch (error) {
    return c.json({ error: '偵測失敗' }, 500);
  }
});

// Analyze user strengths
accessibilityRouter.post('/analyze-strengths', async (c) => {
  try {
    const body = await c.req.json();
    const result = await multimodalService.executeTool('analyze_user_strengths', {
      sessionData: body.sessionData,
      timeRange: body.timeRange,
    });
    
    return c.json(result);
  } catch (error) {
    return c.json({ 
      success: false, 
      recommendations: [],
      error: '分析失敗' 
    }, 500);
  }
});

// Process voice command
accessibilityRouter.post('/voice-command', async (c) => {
  try {
    const body = await c.req.json();
    const result = await multimodalService.executeTool('process_voice_command', {
      command: body.command,
      language: body.language || 'zh-HK',
      context: body.context,
    });
    
    return c.json({ success: true, ...result });
  } catch (error) {
    return c.json({ success: false, error: '命令處理失敗' }, 500);
  }
});

// Save preferences
accessibilityRouter.post('/preferences', async (c) => {
  try {
    const body = await c.req.json();
    // In production, save to database
    console.log('保存偏好設定:', body);
    
    return c.json({ success: true });
  } catch (error) {
    return c.json({ error: '保存失敗' }, 500);
  }
});

export default accessibilityRouter;
