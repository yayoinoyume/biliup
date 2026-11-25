'use client'

import { Checkbox, Input, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import { relayUpdateConfig, type RelayConfigResponse, type RelayUpdateConfigRequest } from '@/app/lib/relay'
import { expectedConfigValues, type RelayConfigFormKey } from './config-expected'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'
import styles from './settings.module.scss'

const { Text } = Typography

const PLATFORM_LABEL: Record<string, string> = {
  YT: 'YouTube',
  TW: 'Twitch',
  NC: 'Niconico',
}

const REASON_OPTIONS = [
  { id: 'idle_recording', label: '挂机 / 录播', risk: '低风险' },
  { id: 'game_mode_off', label: '未开启游戏直播模式', risk: '中风险' },
  { id: 'game_info_violation', label: '游戏内违规信息', risk: '高风险' },
  { id: 'content_unsuitable', label: '直播内容不适宜', risk: '高风险' },
  { id: 'unrecognized', label: '未识别的原因', risk: '高风险' },
] as const

const LOW_RISK_ID = 'idle_recording'

export interface WarningExemptProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}

const sameSet = (left: string[], right: string[]): boolean =>
  JSON.stringify([...left].sort()) === JSON.stringify([...right].sort())

function WarningExemptForm({
  data,
  onSaved,
  onReload,
}: {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}) {
  const [loaded, setLoaded] = useState(data)
  const [selected, setSelected] = useState<string[]>(() =>
    data.warning_hold_exempt.map((entry) => entry.platform + ':' + entry.channel_id),
  )
  const [reasons, setReasons] = useState<string[]>(data.warning_hold_exempt_reasons)
  const [allLowRisk, setAllLowRisk] = useState(data.warning_hold_exempt_all_low_risk)
  const [query, setQuery] = useState('')
  const state = useSettingsSave()

  if (loaded !== data) {
    setLoaded(data)
    setSelected(data.warning_hold_exempt.map((entry) => entry.platform + ':' + entry.channel_id))
    setReasons(data.warning_hold_exempt_reasons)
    setAllLowRisk(data.warning_hold_exempt_all_low_risk)
  }

  const catalog = data.warning_hold_exempt_channels.flatMap((channel) =>
    channel.platforms.map((platform) => ({
      key: platform.platform + ':' + platform.channel_id,
      name: channel.name,
      platform: platform.platform,
      channelId: platform.channel_id,
    })),
  )

  const filtered = data.warning_hold_exempt_channels.filter((channel) =>
    channel.name.toLowerCase().includes(query.trim().toLowerCase()),
  )

  const knownOrderedReasons = REASON_OPTIONS.map((option) => option.id).filter((id) =>
    reasons.includes(id),
  )
  const unknownReasons = data.warning_hold_exempt_reasons.filter(
    (id) => !REASON_OPTIONS.some((option) => option.id === id),
  )
  const orderedReasons = [...knownOrderedReasons, ...unknownReasons]
  const initialKeys = data.warning_hold_exempt.map(
    (entry) => entry.platform + ':' + entry.channel_id,
  )

  const missingKeys = selected.filter((key) => !catalog.some((entry) => entry.key === key))
  const catalogUnavailable =
    data.warning_hold_exempt_channels.length === 0 && data.warning_hold_exempt.length > 0
  const selectedDirty = !sameSet(selected, initialKeys)
  const reasonsDirty = !sameSet(orderedReasons, data.warning_hold_exempt_reasons)
  const allLowRiskDirty = allLowRisk !== data.warning_hold_exempt_all_low_risk
  const dirty = selectedDirty || reasonsDirty || allLowRiskDirty

  const patch: RelayUpdateConfigRequest = {}
  const changed: RelayConfigFormKey[] = []

  if (selectedDirty) {
    const entries = selected
      .map((key) => catalog.find((entry) => entry.key === key))
      .filter((entry): entry is (typeof catalog)[number] => entry !== undefined)
      .map((entry) => ({
        platform: entry.platform,
        channel_id: entry.channelId,
        name: entry.name,
      }))
    patch.warning_hold_exempt = entries
    changed.push('warning_hold_exempt')
  }
  if (reasonsDirty) {
    patch.warning_hold_exempt_reasons = orderedReasons
    changed.push('warning_hold_exempt_reasons')
  }
  if (allLowRiskDirty) {
    patch.warning_hold_exempt_all_low_risk = allLowRisk
    changed.push('warning_hold_exempt_all_low_risk')
  }

  const toggleChannel = (key: string, next: boolean) => {
    setSelected((current) =>
      next ? (current.includes(key) ? current : [...current, key]) : current.filter((item) => item !== key),
    )
  }

  const toggleReason = (id: string, next: boolean) => {
    const nextReasons = next
      ? reasons.includes(id)
        ? reasons
        : [...reasons, id]
      : reasons.filter((item) => item !== id)
    setReasons(nextReasons)
    if (!nextReasons.includes(LOW_RISK_ID)) setAllLowRisk(false)
  }

  const save = async () => {
    if (!dirty) return
    if (catalogUnavailable) {
      Toast.error('频道目录加载失败，已禁止保存以防误清空豁免名单，请刷新重试')
      return
    }
    if (allLowRisk && !orderedReasons.includes(LOW_RISK_ID)) {
      Toast.error('「应用到全部频道」需要先勾选挂机 / 录播这类低风险原因')
      return
    }
    await state.run(
      () =>
        relayUpdateConfig({
          ...patch,
          expected: expectedConfigValues(data, changed),
        }),
      onSaved,
    )
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />

      <div className={styles.field}>
        <span className={styles.fieldLabel}>违规原因</span>
        <div className={styles.checkRow}>
          {REASON_OPTIONS.map((option) => (
            <Checkbox
              key={option.id}
              checked={reasons.includes(option.id)}
              onChange={(event) => toggleReason(option.id, event.target.checked === true)}
            >
              {option.label}
              <Tag size="small" type="light" style={{ marginLeft: 6 }}>
                {option.risk}
              </Tag>
            </Checkbox>
          ))}
        </div>
        <span className={styles.fieldHint}>
          命中选中原因时，告警挂起可以手动续播；没选中的原因仍按默认策略处理。
        </span>
      </div>

      <div className={styles.checkRow}>
        <Checkbox
          checked={allLowRisk}
          disabled={!reasons.includes(LOW_RISK_ID)}
          onChange={(event) => setAllLowRisk(event.target.checked === true)}
        >
          对全部频道应用低风险挂起
        </Checkbox>
        <Text type="tertiary" size="small">
          只有勾选「挂机 / 录播」后才允许开启。
        </Text>
      </div>

      <div className={styles.field}>
        <span className={styles.fieldLabel}>豁免频道</span>
        <Input
          value={query}
          onChange={setQuery}
          placeholder="按频道名搜索"
          aria-label="搜索豁免频道"
        />
        <span className={styles.fieldHint}>
          勾选某个平台的频道身份；名单来自当前 channels.json，已从名单删除的身份保存时会自动剔除。
        </span>
        {catalogUnavailable ? (
          <Text type="danger" size="small">
            频道目录为空（可能加载失败）。当前已选 {data.warning_hold_exempt.length} 个豁免身份，为避免误清空已禁止保存，请刷新重试。
          </Text>
        ) : null}
        {unknownReasons.length > 0 ? (
          <Text type="warning" size="small">
            有 {unknownReasons.length} 个历史违规原因不在当前版本的可选项里，保存时会原样回传。
          </Text>
        ) : null}
      </div>

      <ul className={styles.list}>
        {filtered.map((channel) => (
          <li key={channel.name} className={styles.listRow}>
            <div className={styles.listMain}>
              <Text strong>{channel.name}</Text>
              {channel.platforms.map((platform) => {
                const key = platform.platform + ':' + platform.channel_id
                return (
                  <Checkbox
                    key={key}
                    checked={selected.includes(key)}
                    onChange={(event) => toggleChannel(key, event.target.checked === true)}
                  >
                    {PLATFORM_LABEL[platform.platform] ?? platform.platform}
                  </Checkbox>
                )
              })}
            </div>
          </li>
        ))}
        {filtered.length === 0 ? (
          <li className={styles.listRow}>
            <Text type="tertiary" size="small">
              没有匹配的频道。
            </Text>
          </li>
        ) : null}
      </ul>

      {missingKeys.length > 0 ? (
        <div className={styles.field}>
          <span className={styles.fieldLabel}>已不在频道名单里的选择</span>
          <div className={styles.checkRow}>
            {missingKeys.map((key) => (
              <Tag key={key} size="small" color="orange" closable onClose={() => toggleChannel(key, false)}>
                {key}
              </Tag>
            ))}
          </div>
          <span className={styles.fieldHint}>
            保存时会剔除此类身份；如果还要保留，请先到频道与分区页面补回频道。
          </span>
        </div>
      ) : null}

      <SaveRow
        dirty={dirty}
        saving={state.saving}
        disabled={catalogUnavailable}
        onSave={() => {
          void save()
        }}
        onReset={() => {
          setSelected(initialKeys)
          setReasons(data.warning_hold_exempt_reasons)
          setAllLowRisk(data.warning_hold_exempt_all_low_risk)
          state.clearError()
        }}
        okText="保存豁免设置"
      />
    </>
  )
}

export default function WarningExempt({ data, onSaved, onReload }: WarningExemptProps) {
  return (
    <SettingsCard
      title="违规挂起豁免"
      description="哪些违规原因与频道可以越过告警挂起继续转播；频道身份必须是监控名单里已有的平台 ID。"
    >
      <WarningExemptForm data={data} onSaved={onSaved} onReload={onReload} />
    </SettingsCard>
  )
}
