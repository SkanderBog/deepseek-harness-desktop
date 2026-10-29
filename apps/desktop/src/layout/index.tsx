import { useMount } from '@reause/core'
import { isTauri } from '@tauri-apps/api/core'
import { useThemeAdaptive } from '@/hooks/use-theme-adaptive'
import { store } from '@/store'
import { Webview } from './components/webview'
import { useFrameChrome } from './components/window-chrome'
import '../i18n'

export function App() {
  useThemeAdaptive()
  useFrameChrome()
  useMount(() => {
    if (isTauri())
      void store.setting.refresh().catch(error => console.error('[Setting] failed to load native settings:', error))
  })

  return (
    <div className="flex h-screen w-screen" data-testid="dsh-shell-root">
      <Webview />
    </div>
  )
}
