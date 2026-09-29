import type { ChangeEvent, ReactElement, ReactNode } from 'react'
import type { NotificationSound, TurnCompleteMode } from '../types'
import { Button, Checkbox, Select } from 'dsh-tauri-ui/client'
import { useStore } from 'dsh-tauri/client'
import { useRef, useState } from 'react'
import { locale } from '../locales'
import { readSoundFile } from '../service/sound-file'
import { notificationSettings } from '../store'

/** 一行设置：左侧标题 + 说明，右侧控件，行间以细分隔线收拢成一组。 */
function Row(props: { title: string, hint: string, children: ReactNode }): ReactElement {
  return (
    <div className="flex items-center justify-between gap-[16px] px-[16px] py-[12px] border-b border-border-weak last:border-b-0">
      <div className="flex min-w-0 flex-col">
        <span className="text-[13px] leading-[20px] text-primary">{props.title}</span>
        <span className="text-[12px] leading-[18px] text-tertiary">{props.hint}</span>
      </div>
      <div className="flex flex-none items-center gap-[8px]">{props.children}</div>
    </div>
  )
}

/**
 * 通知设置分区：轮次完成通知 / 启用权限通知 / 启用问题通知 / 通知提示音。
 *
 * 状态读写都走 `notificationSettings` 单例，与通知运行时共用同一份配置。
 */
export function NotificationSettingsSection(): ReactElement {
  locale.useLocale()
  const { turnComplete, approval, question, sound, customSound } = useStore(notificationSettings)
  const [notice, setNotice] = useState<string | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const turnOptions = [
    { value: 'never', label: locale.text('modeNever') },
    { value: 'background', label: locale.text('modeBackground') },
    { value: 'always', label: locale.text('modeAlways') },
  ]
  const soundOptions = [
    { value: 'default', label: locale.text('soundDefault') },
    { value: 'classic', label: locale.text('soundClassic') },
    { value: 'none', label: locale.text('soundNone') },
    { value: 'custom', label: locale.text('soundCustom') },
  ]

  async function pickSound(event: ChangeEvent<HTMLInputElement>): Promise<void> {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file)
      return
    const result = await readSoundFile(file)
    if (result.ok) {
      notificationSettings.setCustomSound(result.dataUrl)
      setNotice(null)
      return
    }
    setNotice(locale.text(result.error === 'too-large' ? 'soundCustomLarge' : 'soundCustomUnreadable'))
  }

  return (
    <div className="flex flex-col gap-[8px]">
      <div className="flex flex-col rounded-[12px] border border-border-weak">
        <Row title={locale.text('turnComplete')} hint={locale.text('turnCompleteHint')}>
          <Select
            options={turnOptions}
            value={turnComplete}
            onChange={next => notificationSettings.setTurnComplete(next as TurnCompleteMode)}
          />
        </Row>
        <Row title={locale.text('approval')} hint={locale.text('approvalHint')}>
          <Checkbox
            checked={approval}
            aria-label={locale.text('approval')}
            onChange={next => notificationSettings.setApproval(next)}
          />
        </Row>
        <Row title={locale.text('question')} hint={locale.text('questionHint')}>
          <Checkbox
            checked={question}
            aria-label={locale.text('question')}
            onChange={next => notificationSettings.setQuestion(next)}
          />
        </Row>
        <Row title={locale.text('sound')} hint={locale.text('soundHint')}>
          <Select
            options={soundOptions}
            value={sound}
            onChange={next => notificationSettings.setSound(next as NotificationSound)}
          />
          {sound === 'custom'
            ? (
                <>
                  <Button type="button" variant="outline" size="sm" onClick={() => fileRef.current?.click()}>
                    {locale.text('soundCustomChoose')}
                  </Button>
                  {customSound
                    ? (
                        <Button type="button" variant="outline" size="sm" onClick={() => notificationSettings.setCustomSound(null)}>
                          {locale.text('soundCustomClear')}
                        </Button>
                      )
                    : null}
                </>
              )
            : null}
        </Row>
      </div>
      <input
        ref={fileRef}
        type="file"
        accept="audio/*"
        hidden
        onChange={(event) => { void pickSound(event) }}
      />
      {notice ? <div className="text-[12px] leading-[18px] text-error" role="alert">{notice}</div> : null}
    </div>
  )
}
