import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useEffect, useState } from 'react'
import './App.css'

const isMacOS = navigator.userAgent.includes('Macintosh')

function App() {
  const [maximized, setMaximized] = useState(false)
  const [fullscreen, setFullscreen] = useState(false)

  // keep:effect 订阅原生窗口状态并在卸载时释放监听。
  useEffect(() => {
    if (!isTauri())
      return

    const appWindow = getCurrentWindow()
    let mounted = true
    const sync = () => {
      void Promise.all([appWindow.isMaximized(), appWindow.isFullscreen()]).then(([maximized, fullscreen]) => {
        if (mounted) {
          setMaximized(maximized)
          setFullscreen(fullscreen)
        }
      }).catch(console.error)
    }
    const listener = appWindow.onResized(sync)
    void listener.then(sync).catch(console.error)
    return () => {
      mounted = false
      void listener.then(unlisten => unlisten()).catch(console.error)
    }
  }, [])

  function handleWindowAction(action: 'minimize' | 'toggleMaximize' | 'close') {
    if (isTauri())
      void getCurrentWindow()[action]().catch(console.error)
  }

  return (
    <div className="shell" data-testid="dsh-shell-root">
      <main className="shell-frame">
        <header className="navbar" data-macos={isMacOS} hidden={isMacOS && fullscreen} data-testid="dsh-navbar-root">
          <div className="navbar-drag-region" data-tauri-drag-region data-testid="dsh-navbar-drag-region" />
          {!isMacOS && (
            <div className="window-controls">
              <button type="button" aria-label="最小化" title="最小化" onClick={() => handleWindowAction('minimize')}>
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M5 12h14" /></svg>
              </button>
              <button type="button" aria-label={maximized ? '还原' : '最大化'} title={maximized ? '还原' : '最大化'} onClick={() => handleWindowAction('toggleMaximize')}>
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  {maximized ? <path d="M8 8V5h11v11h-3M5 8h11v11H5z" /> : <rect x="5" y="5" width="14" height="14" rx="1" />}
                </svg>
              </button>
              <button type="button" className="window-close" aria-label="关闭" title="关闭" onClick={() => handleWindowAction('close')}>
                <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m6 6 12 12M6 18 18 6" /></svg>
              </button>
            </div>
          )}
        </header>
        <div className="shell-content" data-testid="dsh-shell-content" />
      </main>
    </div>
  )
}

export default App
