import { Copy, Minus, Square, Xmark } from '@gravity-ui/icons'
import { Button, Chip } from '@heroui/react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useTranslation } from 'react-i18next'
import { If } from 'react-if-lite'
import { cn } from 'tailwind-variants'
import { hasNativeWindow, IS_MACOS, onDragRegionPointerDown, useMacOSFullscreen, useMaximized } from './window-chrome'

function handleWindowAction(action: 'minimize' | 'maximize' | 'close') {
  if (!hasNativeWindow())
    return
  const appWindow = getCurrentWindow()
  switch (action) {
    case 'minimize':
      void appWindow.minimize()
      break
    case 'maximize':
      void appWindow.toggleMaximize()
      break
    case 'close':
      void appWindow.close()
      break
  }
}

/**
 * Windows/Linux 自绘窗口控制（最小化 / 最大化切换 / 关闭）。
 *
 * 关闭语义由 Rust 按 label 决定（见 src-tauri/src/desktop/builder.rs 的 CloseRequested）：
 * 主窗口按设置驻留托盘或退出，附加窗口直接销毁，这里只在文案上区分两者。
 */
export function WindowControls() {
  const { t } = useTranslation()
  const isMaximized = useMaximized()

  return (
    <>
      <Button
        className="size-7"
        isIconOnly
        size="sm"
        variant="ghost"
        aria-label={t('nav.minimize')}
        onPress={() => { handleWindowAction('minimize') }}
      >
        <Minus />
      </Button>

      <Button
        className="size-7"
        isIconOnly
        size="sm"
        variant="ghost"
        aria-label={t(isMaximized ? 'nav.restore' : 'nav.maximize')}
        onPress={() => { handleWindowAction('maximize') }}
      >
        <If
          cond={isMaximized}
          then={<Copy style={{ width: 14, height: 14 }} />}
          else={<Square style={{ width: 14, height: 14 }} />}
        />
      </Button>

      <Button
        className="size-7 transition-colors enabled:hover:bg-danger/16 enabled:hover:text-danger"
        isIconOnly
        size="sm"
        variant="ghost"
        aria-label={t('nav.close')}
        onPress={() => { handleWindowAction('close') }}
      >
        <Xmark />
      </Button>
    </>
  )
}

export function Navbar() {
  const { t } = useTranslation()
  const isFullscreen = useMacOSFullscreen()

  return (
    <div
      className={cn(
        'relative flex h-11 w-full flex-none select-none items-center gap-0.5 bg-panel',
        {
          'hidden': IS_MACOS && isFullscreen,
          'pl-24 pr-1.5': IS_MACOS && !isFullscreen,
          'px-1.5': !IS_MACOS || isFullscreen,
        },
      )}
      data-testid="dsh-navbar-root"
    >
      <If cond={import.meta.env.DEV}>
        <Chip size="sm" variant="primary" color="warning" className="text-xs text-background ml-1" data-testid="dsh-navbar-dev-chip">
          {t('app.dev_env')}
        </Chip>
      </If>
      <div
        className="min-w-0 flex-1 self-stretch touch-none"
        data-testid="dsh-navbar-drag-region"
        data-tauri-drag-region
        onPointerDown={onDragRegionPointerDown}
      />
      <If cond={!IS_MACOS}>
        <WindowControls />
      </If>
    </div>
  )
}
