import { OverlaysProvider } from '@overlastic/react'
import { QueryClientProvider } from '@tanstack/react-query'
import { getCurrentWindow } from '@tauri-apps/api/window'
import React from 'react'
import ReactDOM from 'react-dom/client'
import { ToastProvider } from './components/toast-provider'
import { queryClient } from './config/client'
import { resolveTheme } from './hooks/use-theme-adaptive'
import { App } from './layout'
import { applyFrameMode, hasNativeWindow } from './layout/components/window-chrome'
import './utils/logger'
import './styles/main.css'

async function bootstrap() {
  const preference = undefined
  document.documentElement.dataset.theme = resolveTheme(
    preference,
    window.matchMedia('(prefers-color-scheme: dark)').matches,
  )
  // 外框模式首帧就要定对：恢复出的最大化/全屏窗口若按常规 border 绘制，会带出
  // 被屏幕边缘裁硬的圆角。此后窗口状态变化由 `useFrameChrome` 订阅维护。
  const framed = (window as Window & { __DSH_FRAME__?: boolean }).__DSH_FRAME__ === true
  if (framed && hasNativeWindow()) {
    try {
      const appWindow = getCurrentWindow()
      const [maximized, fullscreen] = await Promise.all([
        appWindow.isMaximized(),
        appWindow.isFullscreen(),
      ])
      applyFrameMode(maximized, fullscreen)
    }
    catch (error) {
      console.warn('[Frame] failed to sync initial window state:', error)
    }
  }

  ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
    <React.StrictMode>
      <QueryClientProvider client={queryClient}>
        <ToastProvider>
          <OverlaysProvider>
            <App />
          </OverlaysProvider>
        </ToastProvider>
      </QueryClientProvider>
    </React.StrictMode>,
  )
}

void bootstrap()
