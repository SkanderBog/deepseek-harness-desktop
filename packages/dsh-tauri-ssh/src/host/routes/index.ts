import { defineRoutes } from 'dsh-tauri'
import machinesConnect from './machines/connect/post'
import machinesDelete from './machines/delete'
import machinesDisconnect from './machines/disconnect/post'
import machinesEvents from './machines/events/get'
import machinesGet from './machines/get'
import machinesInstall from './machines/install/post'
import machinesPost from './machines/post'
import machinesTest from './machines/test/post'
import sessionRole from './session/role/get'
import settingsGet from './settings/get'
import settingsPost from './settings/post'
import syncApply from './sync/apply/post'
import syncPreview from './sync/preview/get'

export const routes = defineRoutes((disposer) => {
  disposer.get({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/settings' }, settingsGet)
  disposer.post({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/settings' }, settingsPost)
  disposer.get({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/session/role' }, sessionRole)
  disposer.get({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/machines' }, machinesGet)
  disposer.post({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/machines' }, machinesPost)
  disposer.delete({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/machines' }, machinesDelete)
  disposer.post({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/machines/test' }, machinesTest)
  disposer.post({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/machines/connect' }, machinesConnect)
  disposer.post({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/machines/disconnect' }, machinesDisconnect)
  disposer.post({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/machines/install' }, machinesInstall)
  disposer.get({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/machines/events' }, machinesEvents)
  disposer.get({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/sync/preview' }, syncPreview)
  disposer.post({ kind: 'exact', path: '/api/desktop/dsh-tauri-ssh/sync/apply' }, syncApply)
})
