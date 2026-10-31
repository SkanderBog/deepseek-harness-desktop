import { useWatch } from '@reause/core'
import { invoke } from '@tauri-apps/api/core'
import { getCurrentWindow, PhysicalSize } from '@tauri-apps/api/window'
import { PET_BUBBLE_MIN_WIDTH, PET_WINDOW_PAD_X, PET_WINDOW_PAD_Y } from '../constants'

export function usePetWindowSize(width: number, aspect: number, visible: boolean): void {
  useWatch([width, aspect, visible], ([nextWidth, nextAspect, nextVisible]) => {
    if (!nextVisible)
      return
    const scale = globalThis.devicePixelRatio || 1
    const windowWidth = Math.max(nextWidth + PET_WINDOW_PAD_X, PET_BUBBLE_MIN_WIDTH)
    const windowHeight = nextWidth * nextAspect + PET_WINDOW_PAD_Y
    void getCurrentWindow()
      .setSize(new PhysicalSize(Math.round(windowWidth * scale), Math.round(windowHeight * scale)))
      .then(() => {
        void invoke<void>('move_pet_window', { deltaX: 0, deltaY: 0 }).catch(() => {})
      })
      .catch((error) => {
        console.warn('[pet] PET_WINDOW_RESIZE_FAILED:', error)
      })
  }, { immediate: true })
}
