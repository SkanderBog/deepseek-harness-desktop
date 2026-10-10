import type { ClientContext, PanelHandle } from 'dsh-tauri/client'
import { Panel } from 'dsh-tauri-ui/client'
import { definePanel, defineRegister } from 'dsh-tauri/client'
import { SchedulerNavIcon } from '../components/scheduler-nav-icon'
import { SchedulerPanel } from '../components/scheduler-panel'
import { PANEL_ACTION_ORDER, PANEL_ID, REFRESH_INTERVAL_MS } from '../constants'
import { locale } from '../locales'
import { loadScheduler } from '../service/scheduler'
import { store } from '../store'

export const panelFeature = defineRegister<ClientContext>((controller, ctx, adapter) => {
  const holder: { current?: PanelHandle } = {}
  void loadScheduler(true)
  controller.interval(() => void loadScheduler(false), REFRESH_INTERVAL_MS)
  controller.listen('visibilitychange', () => {
    if (document.visibilityState === 'visible')
      void loadScheduler(false)
  })
  controller.listenWindow('focus', () => void loadScheduler(false))
  const sessionList = adapter.sessionList()
  if (sessionList) {
    controller.add(sessionList.subscribe(() => {
      const current = adapter.sessionList()?.current
      if (current)
        store.scheduler.markSessionRead(current)
    }))
  }
  holder.current = definePanel(ctx, {
    id: PANEL_ID,
    order: PANEL_ACTION_ORDER,
    locale: locale.NS,
    label: () => locale.text('scheduler'),
    icon: props => <SchedulerNavIcon size={props.size} />,
    render: () => (
      <Panel>
        <SchedulerPanel
          t={locale.text}
          onViaChat={() => {
            store.prefill.set(locale.text('chatPrompt'))
            holder.current?.close()
          }}
        />
      </Panel>
    ),
  })
  controller.add(holder.current.dispose)
})
