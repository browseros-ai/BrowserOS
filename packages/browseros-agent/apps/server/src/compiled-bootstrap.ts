import { installNativeAddonGuard } from './lib/native-addon-guard'

installNativeAddonGuard()
await import('./index')
