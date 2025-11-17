# BlockBrowser Mobile

AI-powered mobile browser with privacy-first features, built with React Native and Expo SDK 54.

## Features

### 🌐 Full-Featured Browser
- Multi-tab browsing with WebView
- Address bar with URL validation
- Browsing history and bookmarks
- Back/forward navigation
- Page loading progress
- Incognito mode support

### 🤖 AI Integration
- Multiple AI provider support (OpenAI, Anthropic, Google Gemini, Ollama, OpenRouter)
- Real-time streaming responses
- Chat session management
- Provider switching
- Secure API key storage

### 🎨 Modern UI/UX
- Liquid Glass effects on iOS (using @callstack/liquid-glass)
- Dark and light mode support
- Smooth animations and transitions
- Native blur effects
- Responsive design

### 🔒 Privacy & Security
- Ad blocking
- Tracker blocking
- JavaScript control
- Secure API key storage (iOS Keychain, Android Keystore)
- Clear history on exit
- Incognito mode

## Tech Stack

- **Framework**: React Native 0.81
- **SDK**: Expo 54
- **TypeScript**: 5.x
- **State Management**: Zustand with AsyncStorage persistence
- **Navigation**: Expo Router v6
- **WebView**: react-native-webview
- **UI Effects**: @callstack/liquid-glass
- **Styling**: NativeWind (Tailwind CSS)
- **Security**: expo-secure-store

## Prerequisites

- Node.js 18+ and npm/yarn
- iOS: Xcode 15+ and CocoaPods
- Android: Android Studio and JDK 17+
- Expo CLI: `npm install -g expo-cli`

## Installation

1. Clone the repository:
```bash
git clone <repository-url>
cd blockbrowser-mobile
```

2. Install dependencies:
```bash
npm install
# or
yarn install
```

3. Install iOS pods (iOS only):
```bash
cd ios && pod install && cd ..
```

## Running the App

### Development Mode

Start the Expo development server:
```bash
npm start
# or
yarn start
```

Then choose your platform:
- Press `i` for iOS simulator
- Press `a` for Android emulator
- Scan QR code with Expo Go app for physical device

### iOS
```bash
npm run ios
# or
yarn ios
```

### Android
```bash
npm run android
# or
yarn android
```

## Project Structure

```
blockbrowser-mobile/
├── app/                      # Expo Router screens
│   ├── _layout.tsx          # Root layout
│   └── (tabs)/              # Tab navigation
│       ├── _layout.tsx      # Tab layout
│       ├── index.tsx        # Browser screen
│       ├── ai.tsx           # AI chat screen
│       ├── tabs.tsx         # Tab manager
│       └── settings.tsx     # Settings screen
├── components/              # Reusable components
│   ├── browser/            # Browser components
│   │   ├── WebViewBrowser.tsx
│   │   ├── AddressBar.tsx
│   │   └── TabCard.tsx
│   ├── ai/                 # AI chat components
│   │   ├── ChatInterface.tsx
│   │   ├── MessageBubble.tsx
│   │   ├── ProviderSelector.tsx
│   │   └── StreamingIndicator.tsx
│   └── settings/           # Settings components
│       ├── ProviderCard.tsx
│       └── ApiKeyInput.tsx
├── store/                  # Zustand state stores
│   ├── browser-store.ts   # Browser state
│   ├── ai-store.ts        # AI chat state
│   └── settings-store.ts  # Settings state
├── lib/                    # Utilities and libraries
│   ├── ai/                # AI provider integration
│   │   └── providers/
│   │       ├── base.ts
│   │       ├── openai.ts
│   │       ├── anthropic.ts
│   │       └── factory.ts
│   ├── storage/           # Secure storage
│   │   └── secure-store.ts
│   └── utils/             # Utility functions
│       └── url-validator.ts
└── constants/             # App constants
    ├── Colors.ts
    └── Providers.ts
```

## Configuration

### AI Providers

Configure AI providers in the Settings screen:

1. **OpenAI**
   - Get API key from: https://platform.openai.com/api-keys
   - Models: GPT-4, GPT-3.5 Turbo

2. **Anthropic Claude**
   - Get API key from: https://console.anthropic.com/
   - Models: Claude 3 Opus, Claude 3 Sonnet

3. **Google Gemini**
   - Get API key from: https://makersuite.google.com/app/apikey
   - Models: Gemini Pro

4. **OpenRouter**
   - Get API key from: https://openrouter.ai/keys
   - Access to multiple models

5. **Ollama** (Local)
   - Install Ollama: https://ollama.ai
   - Configure base URL to your Ollama instance

### Privacy Settings

- **Block Ads**: Enable ad blocking for cleaner browsing
- **Block Trackers**: Prevent tracking scripts from running
- **Enable JavaScript**: Control JavaScript execution
- **Clear on Exit**: Automatically clear history when app closes

## Development

### Adding New AI Providers

1. Create a new provider class in `lib/ai/providers/`:
```typescript
import { BaseAIProvider } from './base';

export class NewProvider extends BaseAIProvider {
  async chat(messages, model, apiKey, options) {
    // Implementation
  }
}
```

2. Update the factory in `lib/ai/providers/factory.ts`

3. Add provider configuration in `constants/Providers.ts`

### Adding New Components

Follow the existing component structure:
- Use TypeScript for type safety
- Implement liquid-glass effects for iOS
- Support both light and dark modes
- Add proper error handling

## Building for Production

### iOS

1. Configure app signing in Xcode
2. Build:
```bash
eas build --platform ios
```

### Android

1. Configure signing in `android/app/build.gradle`
2. Build:
```bash
eas build --platform android
```

## Features Roadmap

- [ ] Bookmark syncing
- [ ] Extensions support
- [ ] Download manager
- [ ] Reader mode
- [ ] Password manager
- [ ] VPN integration
- [ ] Custom search engines
- [ ] Tab groups
- [ ] Picture-in-picture mode
- [ ] Desktop sync

## Security

- API keys are stored securely using:
  - iOS: Keychain Services
  - Android: Android Keystore
- No sensitive data in AsyncStorage
- HTTPS enforcement
- Secure WebView configuration
- No telemetry or tracking

## Troubleshooting

### iOS Issues

**Liquid Glass not working:**
- Ensure iOS 16+ is installed
- Check that the device supports the effect
- Fallback UI will be used on older devices

**Build errors:**
```bash
cd ios && pod deintegrate && pod install && cd ..
```

### Android Issues

**WebView not loading:**
- Enable JavaScript in settings
- Check network permissions in AndroidManifest.xml

**Gradle build errors:**
```bash
cd android && ./gradlew clean && cd ..
```

## Contributing

1. Fork the repository
2. Create a feature branch: `git checkout -b feature/amazing-feature`
3. Commit your changes: `git commit -m 'Add amazing feature'`
4. Push to the branch: `git push origin feature/amazing-feature`
5. Open a Pull Request

## License

MIT License - see LICENSE file for details

## Acknowledgments

- [Expo](https://expo.dev) - Framework and tooling
- [Callstack Liquid Glass](https://github.com/callstack/liquid-glass) - iOS frosted glass effects
- [React Native WebView](https://github.com/react-native-webview/react-native-webview) - WebView component
- [Zustand](https://github.com/pmndrs/zustand) - State management

## Support

For issues, questions, or contributions:
- Open an issue on GitHub
- Check existing documentation
- Review closed issues for solutions

---

Built with ❤️ using React Native and Expo
