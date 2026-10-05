// @vitest-environment jsdom
import { OverlaysProvider, useOverlay } from '@overlastic/react'
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { StrictMode } from 'react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { DesktopUpdateDialog } from './update'

const { desktopUpdater } = vi.hoisted(() => ({
  desktopUpdater: {
    updateInfo: null as null | { currentVersion: string, version: string, tag: string, downloaded: boolean, path: string },
    downloading: false,
    downloadProgress: 0,
    downloadAndOpen: vi.fn(),
    openInstaller: vi.fn(),
  },
}))

vi.mock('@/store', () => ({ store: { desktopUpdater } }))
vi.mock('valtio-define', () => ({ useStore: (value: unknown) => value }))
vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }))

function Launcher() {
  const open = useOverlay(DesktopUpdateDialog)
  function handleOpen() {
    void open().catch(() => {})
  }
  return <button onClick={handleOpen}>Open update</button>
}

function openDialog() {
  render(<StrictMode><OverlaysProvider><Launcher /></OverlaysProvider></StrictMode>)
  fireEvent.click(screen.getByText('Open update'))
}

beforeEach(() => {
  desktopUpdater.updateInfo = {
    currentVersion: '0.21.1',
    version: '0.21.2',
    tag: 'v0.21.2',
    downloaded: false,
    path: 'C:/installer.exe',
  }
  desktopUpdater.downloading = false
  desktopUpdater.downloadProgress = 0
  desktopUpdater.downloadAndOpen.mockReset()
  desktopUpdater.openInstaller.mockReset()
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('desktop update dialog', () => {
  it('hides the footer actions while downloading and keeps the progress visible', async () => {
    desktopUpdater.downloading = true
    desktopUpdater.downloadProgress = 42
    openDialog()
    await screen.findByText('update.desktop_downloading')
    expect(screen.queryByRole('button', { name: 'update.later' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'update.now' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'update.open_installer' })).toBeNull()
  })

  it('offers the download action once no download is in flight', async () => {
    openDialog()
    expect((await screen.findByRole('button', { name: 'update.now' })).textContent).toBe('update.now')
    expect(screen.queryByRole('button', { name: 'update.later' })).not.toBeNull()
  })

  it('offers the installer action once the package is downloaded', async () => {
    desktopUpdater.updateInfo!.downloaded = true
    openDialog()
    expect((await screen.findByRole('button', { name: 'update.open_installer' })).textContent).toBe('update.open_installer')
  })

  it('opens the installer instead of downloading when the package already exists', async () => {
    desktopUpdater.updateInfo!.downloaded = true
    openDialog()
    fireEvent.click(await screen.findByRole('button', { name: 'update.open_installer' }))
    expect(desktopUpdater.openInstaller).toHaveBeenCalledExactlyOnceWith('C:/installer.exe')
    expect(desktopUpdater.downloadAndOpen).not.toHaveBeenCalled()
  })
})
