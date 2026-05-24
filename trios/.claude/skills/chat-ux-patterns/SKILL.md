## Chat UI/UX Best Practices

### 1. Message Bubble Design
- User: Right, accent color, sharp top-left
- Agent: Left, glassmorphism, rounded all
- Code: Full width, dark bg, monospace

### 2. Streaming Animation
Character-by-character reveal with 16ms delay

### 3. Typing Indicator
Three dots with staggered bounce 0.6s

### 4. Auto-Scroll
ScrollViewReader + scrollTo bottom on new message

### 5. Session Vitality UI
- Header: Queen status + elapsed time
- Token counter with progress bar
- Auto-save indicator

### 6. Performance
LazyVStack, image lazy loading, 30 FPS throttle