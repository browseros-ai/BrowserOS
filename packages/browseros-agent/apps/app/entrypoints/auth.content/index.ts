import { env } from '@/lib/env'
import {
  RuntimeMessageType,
  sendRuntimeMessage,
} from '@/lib/messaging/runtime/runtimeMessages'

export default defineContentScript({
  matches: [`${env.VITE_PUBLIC_BROWSEROS_API}/home`],
  runAt: 'document_start',
  main() {
    window.addEventListener('message', (event) => {
      if (event.data?.type === 'AUTH_SUCCESS') {
        void sendRuntimeMessage(RuntimeMessageType.authSuccess)
      }
    })
  },
})
