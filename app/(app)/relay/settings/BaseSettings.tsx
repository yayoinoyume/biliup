'use client'

import { Checkbox, InputNumber, Tag, Toast } from '@douyinfe/semi-ui'
import { useState } from 'react'
import {
  relayUpdateConfig,
  type RelayConfigResponse,
  type RelayUpdateConfigRequest,
} from '@/app/lib/relay'
import { expectedConfigValues, type RelayConfigFormKey } from './config-expected'
import { SaveErrorBanner, SaveRow, SettingsCard, SettingsField, useSettingsSave } from './shared'
import styles from './settings.module.scss'

export interface BaseSettingsProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}

const readNumber = (value: number | string, fallback: number): number => {
  const parsed = typeof value === 'number' ? value : Number(value)
  return Number.isFinite(parsed) ? Math.trunc(parsed) : fallback
}

export default function BaseSettings({ data, onSaved, onReload }: BaseSettingsProps) {
  const initialLolInterval = data.lol_monitor_interval ?? 1
  const [interval, setInterval] = useState(data.interval)
  const [autoStop, setAutoStop] = useState(data.auto_stop_after_source_end)
  const [autoStopHours, setAutoStopHours] = useState(data.auto_stop_threshold_hours)
  const [autoCover, setAutoCover] = useState(data.auto_cover)
  const [showPriority, setShowPriority] = useState(data.show_priority_channel)
  const [showTwitch, setShowTwitch] = useState(data.show_twitch)
  const [showNiconico, setShowNiconico] = useState(data.show_niconico)
  const [youtubeRss, setYoutubeRss] = useState(data.youtube_rss_enabled)
  const [danmakuCommand, setDanmakuCommand] = useState(data.bilibili.enable_danmaku_command)
  const [holodexGate, setHolodexGate] = useState(data.holodex_monitor_gate)
  const [lolMonitor, setLolMonitor] = useState(data.enable_lol_monitor)
  const [lolInterval, setLolInterval] = useState(initialLolInterval)
  const state = useSettingsSave()

  const patch: RelayUpdateConfigRequest = {}
  const changed: RelayConfigFormKey[] = []

  if (interval !== data.interval) {
    patch.interval = interval
    changed.push('interval')
  }
  if (autoStop !== data.auto_stop_after_source_end) {
    patch.auto_stop_after_source_end = autoStop
    changed.push('auto_stop_after_source_end')
  }
  if (autoStopHours !== data.auto_stop_threshold_hours) {
    patch.auto_stop_threshold_hours = autoStopHours
    changed.push('auto_stop_threshold_hours')
  }
  if (autoCover !== data.auto_cover) {
    patch.auto_cover = autoCover
    changed.push('auto_cover')
  }
  if (showPriority !== data.show_priority_channel) {
    patch.show_priority_channel = showPriority
    changed.push('show_priority_channel')
  }
  if (showTwitch !== data.show_twitch) {
    patch.show_twitch = showTwitch
    changed.push('show_twitch')
  }
  if (showNiconico !== data.show_niconico) {
    patch.show_niconico = showNiconico
    changed.push('show_niconico')
  }
  if (youtubeRss !== data.youtube_rss_enabled) {
    patch.youtube_rss_enabled = youtubeRss
    changed.push('youtube_rss_enabled')
  }
  if (danmakuCommand !== data.bilibili.enable_danmaku_command) {
    patch.enable_danmaku_command = danmakuCommand
    changed.push('enable_danmaku_command')
  }
  if (holodexGate !== data.holodex_monitor_gate) {
    patch.holodex_monitor_gate = holodexGate
    changed.push('holodex_monitor_gate')
  }
  if (lolMonitor !== data.enable_lol_monitor) {
    patch.enable_lol_monitor = lolMonitor
    changed.push('enable_lol_monitor')
  }
  if (lolInterval !== initialLolInterval) {
    patch.lol_monitor_interval = lolInterval
    changed.push('lol_monitor_interval')
  }

  const dirty = changed.length > 0

  const reset = () => {
    setInterval(data.interval)
    setAutoStop(data.auto_stop_after_source_end)
    setAutoStopHours(data.auto_stop_threshold_hours)
    setAutoCover(data.auto_cover)
    setShowPriority(data.show_priority_channel)
    setShowTwitch(data.show_twitch)
    setShowNiconico(data.show_niconico)
    setYoutubeRss(data.youtube_rss_enabled)
    setDanmakuCommand(data.bilibili.enable_danmaku_command)
    setHolodexGate(data.holodex_monitor_gate)
    setLolMonitor(data.enable_lol_monitor)
    setLolInterval(initialLolInterval)
    state.clearError()
  }

  const save = async () => {
    if (!dirty) return
    if (interval < 1 || interval > 3600) {
      Toast.error('监听间隔需在 1 到 3600 秒之间')
      return
    }
    if (autoStopHours < 1 || autoStopHours > 168) {
      Toast.error('自动停播阈值需在 1 到 168 小时之间')
      return
    }
    if (lolInterval < 1 || lolInterval > 3600) {
      Toast.error('LOL 监听间隔需在 1 到 3600 秒之间')
      return
    }
    const ok = await state.run(
      () =>
        relayUpdateConfig({
          ...patch,
          expected: expectedConfigValues(data, changed),
        }),
      onSaved,
    )
    if (!ok) return
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />
      <SettingsCard
        title="基础设置"
        description="监听间隔、开播/停播行为与主面板显示哪些平台。保存时只提交改过的字段，并带上原始值做冲突校验。"
        extra={
          <Tag size="small" type="light">
            配置版本 {data.secret_revision}
          </Tag>
        }
      >
        <div className={styles.formGrid}>
          <SettingsField label="监听间隔（秒）" hint="轮询直播状态的间隔，允许 1–3600 秒。">
            <InputNumber
              value={interval}
              min={1}
              max={3600}
              precision={0}
              style={{ width: '100%' }}
              onNumberChange={(value) => setInterval(readNumber(value, 1))}
              aria-label="监听间隔"
            />
          </SettingsField>
          <SettingsField label="自动停播阈值（小时）" hint="画面来源结束后等待多久自动停播，允许 1–168 小时。">
            <InputNumber
              value={autoStopHours}
              min={1}
              max={168}
              precision={0}
              style={{ width: '100%' }}
              disabled={!autoStop}
              onNumberChange={(value) => setAutoStopHours(readNumber(value, 1))}
              aria-label="自动停播阈值"
            />
          </SettingsField>
          <SettingsField label="LOL 监听间隔（秒）" hint="启用 LOL 监控时的轮询间隔，允许 1–3600 秒。">
            <InputNumber
              value={lolInterval}
              min={1}
              max={3600}
              precision={0}
              style={{ width: '100%' }}
              disabled={!lolMonitor}
              onNumberChange={(value) => setLolInterval(readNumber(value, 1))}
              aria-label="LOL 监听间隔"
            />
          </SettingsField>
        </div>

        <div className={styles.checkRow}>
          <Checkbox
            checked={autoStop}
            onChange={(event) => setAutoStop(event.target.checked === true)}
          >
            画面来源结束后自动停播
          </Checkbox>
          <Checkbox checked={autoCover} onChange={(event) => setAutoCover(event.target.checked === true)}>
            开播时自动更新封面
          </Checkbox>
          <Checkbox
            checked={danmakuCommand}
            onChange={(event) => setDanmakuCommand(event.target.checked === true)}
          >
            启用弹幕命令
          </Checkbox>
          <Checkbox
            checked={holodexGate}
            onChange={(event) => setHolodexGate(event.target.checked === true)}
          >
            Holodex 监听闸门
          </Checkbox>
          <Checkbox
            checked={lolMonitor}
            onChange={(event) => setLolMonitor(event.target.checked === true)}
          >
            启用 LOL 监控
          </Checkbox>
        </div>

        <div className={styles.checkRow}>
          <Checkbox
            checked={showPriority}
            onChange={(event) => setShowPriority(event.target.checked === true)}
          >
            主面板显示优先频道
          </Checkbox>
          <Checkbox
            checked={showTwitch}
            onChange={(event) => setShowTwitch(event.target.checked === true)}
          >
            主面板显示 Twitch
          </Checkbox>
          <Checkbox
            checked={showNiconico}
            onChange={(event) => setShowNiconico(event.target.checked === true)}
          >
            主面板显示 Niconico
          </Checkbox>
          <Checkbox
            checked={youtubeRss}
            onChange={(event) => setYoutubeRss(event.target.checked === true)}
          >
            启用 YouTube RSS 发现
          </Checkbox>
        </div>

        <SaveRow
          dirty={dirty}
          saving={state.saving}
          onSave={() => {
            void save()
          }}
          onReset={reset}
          okText="保存基础设置"
        />
      </SettingsCard>
    </>
  )
}
