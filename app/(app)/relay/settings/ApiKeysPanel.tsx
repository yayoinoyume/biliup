'use client'

import { Progress, Tag, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayUpdateConfig,
  relayYoutubeKeys,
  type RelayConfigResponse,
  type RelayUpdateConfigRequest,
  type RelayYoutubeKeyStatusEntry,
} from '@/app/lib/relay'
import { formatIsoDateTime, relaySecretPatch } from '@/app/lib/relay-format'
import MaskedSecretInput from '@/app/ui/relay/MaskedSecretInput'
import RelayError from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import { expectedConfigValues, type RelayConfigFormKey } from './config-expected'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'
import styles from './settings.module.scss'

const { Text } = Typography

const KEY_STATE_LABEL: Record<RelayYoutubeKeyStatusEntry['state'], string> = {
  usable: '可用',
  exhausted: '额度用尽',
  rejected: '被拒绝',
}

const KEY_STATE_COLOR: Record<RelayYoutubeKeyStatusEntry['state'], 'green' | 'orange' | 'red'> = {
  usable: 'green',
  exhausted: 'orange',
  rejected: 'red',
}

export interface ApiKeysPanelProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}

export default function ApiKeysPanel({ data, onSaved, onReload }: ApiKeysPanelProps) {
  const pool = useSWR('relay:youtube-keys', relayYoutubeKeys, { revalidateOnFocus: false })
  const [apiKey, setApiKey] = useState('')
  const [clear, setClear] = useState(false)
  const state = useSettingsSave()
  const status = pool.data
  const keys = status?.keys ?? []

  const decision = relaySecretPatch(apiKey, data.youtube_api_key_configured)
  const patch: RelayUpdateConfigRequest = {}
  const changed: RelayConfigFormKey[] = []

  if (clear) {
    patch.clear_youtube_api_key = true
    changed.push('clear_youtube_api_key')
  } else if (decision.secret !== undefined) {
    patch.youtube_api_key = decision.secret
    changed.push('youtube_api_key')
  }

  const dirty = changed.length > 0
  const remaining = status?.remaining_fraction
  const percent =
    typeof remaining === 'number' ? Math.round(Math.min(1, Math.max(0, remaining)) * 100) : null

  const save = async () => {
    if (!dirty) return
    const ok = await state.run(async () => {
      const message = await relayUpdateConfig({
        ...patch,
        expected: expectedConfigValues(data, changed),
        expected_secret_revision: data.secret_revision,
      })
      await onSaved()
      await pool.mutate()
      return message
    })
    if (ok) {
      setApiKey('')
      setClear(false)
    }
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />
      <SettingsCard
        title="YouTube 密钥池"
        description="每个密钥的配额与状态只回指纹，不回密钥本体；额度与重置时间来自 YouTube Data API 的响应。"
        extra={
          <Tag size="small" color={status?.configured ? 'green' : 'grey'}>
            {status?.configured ? '已配置' : '未配置'}
          </Tag>
        }
      >
        {pool.error ? (
          <RelayError
            error={pool.error}
            title="密钥池状态加载失败"
            onRetry={() => {
              void pool.mutate()
            }}
          />
        ) : null}

        {status?.configured ? (
          <div className={styles.keyList}>
            <div className={styles.actions}>
              {percent !== null ? (
                <div style={{ flex: 1, minWidth: 180 }}>
                  <Text type="tertiary" size="small">
                    总剩余额度 {percent}%
                  </Text>
                  <Progress percent={percent} showInfo={false} />
                </div>
              ) : null}
              {typeof status.budget_per_key === 'number' ? (
                <Text type="tertiary" size="small">
                  每个密钥配额 {status.budget_per_key}
                </Text>
              ) : null}
              {status.resets_at ? (
                <Text type="tertiary" size="small">
                  额度重置 {formatIsoDateTime(status.resets_at)}
                </Text>
              ) : null}
            </div>
            {keys.map((entry) => (
              <div key={entry.fingerprint} className={styles.keyRow}>
                <Text className={relayStyles.mono}>{entry.fingerprint}</Text>
                <Text type="tertiary" size="small">
                  已用 {entry.used}
                </Text>
                <Text type="tertiary" size="small">
                  {typeof status.budget_per_key === 'number'
                    ? '剩余 ' + Math.max(0, status.budget_per_key - entry.used)
                    : '剩余未知'}
                </Text>
                <Tag size="small" color={KEY_STATE_COLOR[entry.state]}>
                  {KEY_STATE_LABEL[entry.state]}
                </Tag>
              </div>
            ))}
            {keys.length === 0 ? (
              <Text type="tertiary" size="small">
                状态接口没有返回密钥条目；可以到实时日志确认配额查询是否失败。
              </Text>
            ) : null}
          </div>
        ) : (
          <Text type="tertiary" size="small">
            还没有配置 YouTube API Key。密钥池用于发现频道直播与读取配额，YouTube 监听依赖它。
          </Text>
        )}

        <MaskedSecretInput
          id="relay-youtube-api-key"
          label="YouTube API Key"
          value={apiKey}
          onChange={setApiKey}
          configured={data.youtube_api_key_configured}
          multiline
          rows={3}
          clearRequested={clear}
          onClearChange={setClear}
          placeholder={
            data.youtube_api_key_configured
              ? '已保存，留空表示不修改；多个密钥可一行一个'
              : '填写 API Key；多个密钥可一行一个'
          }
        />
        <SaveRow
          dirty={dirty}
          saving={state.saving}
          onSave={() => {
            void save()
          }}
          onReset={() => {
            setApiKey('')
            setClear(false)
            state.clearError()
          }}
          okText="保存 YouTube 密钥"
        />
      </SettingsCard>
    </>
  )
}
