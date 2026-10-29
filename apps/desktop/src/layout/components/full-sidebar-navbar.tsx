import { LayoutHeader } from '@gravity-ui/icons'
import { If } from 'react-if-lite'
import { cn } from 'tailwind-variants'
import { useDshStyle } from '@/hooks/use-dsh-style'
import { WindowControls } from './navbar'
import { IS_MACOS, onDragRegionPointerDown, useMacOSFullscreen } from './window-chrome'

export function FullSidebarNavbar() {
  const isFullscreen = useMacOSFullscreen()
  const [dshStyle] = useDshStyle()

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
      style={{ background: dshStyle.sidebar?.background }}
      data-testid="dsh-full-sidebar-navbar-root"
    >
      <span
        className="pointer-events-none flex size-7 flex-none items-center justify-center text-foreground/40"
        aria-hidden="true"
        data-testid="dsh-full-sidebar-navbar-mark"
      >
        <LayoutHeader />
      </span>

      <div
        className="min-w-0 flex-1 self-stretch touch-none"
        data-testid="dsh-full-sidebar-navbar-drag-region"
        data-tauri-drag-region
        onPointerDown={onDragRegionPointerDown}
      />

      <div className="absolute" style={dshStyle.marked || {}} />

      <If cond={!IS_MACOS}>
        <WindowControls />
      </If>
    </div>
  )
}
