import { desktopUpdater } from './modules/desktop-updater'
import { harness } from './modules/harness'
import { harnessUpdater } from './modules/harness-updater'
import { plugins } from './modules/plugins'
import { preinstall } from './modules/preinstall'
import { recovery } from './modules/recovery'
import { setting } from './modules/setting'

export const store = {
  harness,
  harnessUpdater,
  plugins,
  preinstall,
  recovery,
  setting,
  desktopUpdater,
}
