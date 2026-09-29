import type { ChangeEvent, ReactElement, ReactNode } from 'react'
import type { SoundPlayer } from '../service/sound'
import type { NotificationSound, TurnCompleteMode } from '../types'
import { Button, Checkbox, Select } from 'dsh-tauri-ui/client'
import { useStore } from 'dsh-tauri/client'
import { useEffect, useRef, useState } from 'react'
import { locale } from '../locales'
import { createSoundPlayer } from '../service/sound'
import { requestBuiltinSounds } from '../service/sound-assets'
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
  const playerRef = useRef<SoundPlayer | null>(null)

  useEffect(() => {
    // 试听要在设置页里立刻出声：先向壳层索要内置音资源（data URL），再建播放器。
    requestBuiltinSounds()
    const player = createSoundPlayer()
    playerRef.current = player
    return () => {
      playerRef.current = null
      player.dispose()
    }
  }, [])

  /** 试听一次当前选择；`none` 与「还没选文件的自定义」不发声。 */
  function preview(next: NotificationSound, custom: string | null): void {
    if (next === 'none' || (next === 'custom' && !custom))
      return
    playerRef.current?.play(next, custom)
  }

  function pickSoundOption(next: NotificationSound): void {
    notificationSettings.setSound(next)
    preview(next, notificationSettings.$state.customSound)
  }

  async function pickSound(event: ChangeEvent<HTMLInputElement>): Promise<void> {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file)
      return
    const result = await readSoundFile(file)
    if (result.ok) {
      notificationSettings.setCustomSound(result.dataUrl)
      setNotice(null)
      // 选完文件直接试听，用户不用等下一次通知才知道选对了没有。
      preview('custom', result.dataUrl)
      return
    }
    setNotice(locale.text(result.error === 'too-large' ? 'soundCustomLarge' : 'soundCustomUnreadable'))
  }

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

  return (
    <div className="flex flex-col gap-[16px]">
      {/* 官方分区只渲染注册项里的内容（壳层不画标题/描述），所以页头由分区自己给。 */}
      <div className="flex flex-col gap-[4px]">
        <h1 className="m-0 text-[24px] leading-[32px] font-semibold text-primary">{locale.text('sectionTitle')}</h1>
        <p className="m-0 text-[13px] leading-[20px] text-secondary">{locale.text('sectionDescription')}</p>
      </div>
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
              onChange={next => pickSoundOption(next as NotificationSound)}
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
    </div>
  )
}
