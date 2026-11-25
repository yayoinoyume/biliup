'use client'

import { useState } from 'react'
import { Button, InputNumber, Switch, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import useSWR from 'swr'
import { useMe } from '@/app/lib/use-me'
import { relayGetFfmpegCache, relayUpdateFfmpegCache } from '@/app/lib/relay'
import type { RelayCropPlatform } from '@/app/lib/relay'
import { relayErrorMessage } from '@/app/ui/relay/RelayError'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

const LATENCY_MIN = 1
const LATENCY_MAX = 60

const CACHE_KEY = (platform: RelayCropPlatform) => ['relay:ffmpeg-cache', platform]

type CacheDraft = { enabled: boolean; latency: number | string }

export default function HlsCacheEditor({ platform }: { platform: RelayCropPlatform }) {
  const { can } = useMe()
  const canEdit = can('relay.config')
  const { data, error, isLoading, mutate } = useSWR(CACHE_KEY(platform), () => relayGetFfmpegCache(platform), {
    revalidateOnFocus: false,
  })
  const [draft, setDraft] = useState<CacheDraft | null>(null)
  const [saving, setSaving] = useState(false)

  const loaded = data !== undefined
  const server: CacheDraft | null = loaded ? { enabled: data.enabled, latency: data.latency_secs } : null
  const value = draft ?? server
  const latencyNumber =
    value === null
      ? Number.NaN
      : typeof value.latency === 'number'
        ? value.latency
        : Number(String(value.latency).trim())
  const latencyValid =
    Number.isInteger(latencyNumber) && latencyNumber >= LATENCY_MIN && latencyNumber <= LATENCY_MAX
  const latencyHint =
    loaded && value !== null && !latencyValid
      ? String(value.latency).trim() === ''
        ? '请填写延迟秒数'
        : `延迟必须是 ${LATENCY_MIN} 到 ${LATENCY_MAX} 之间的整数秒`
      : null

  const save = async () => {
    if (!latencyValid) {
      Toast.error(`延迟必须是 ${LATENCY_MIN} 到 ${LATENCY_MAX} 之间的整数秒`)
      return
    }
    setSaving(true)
    try {
      const message = await relayUpdateFfmpegCache({
        platform,
        enabled: value?.enabled ?? false,
        latency_secs: latencyNumber,
      })
      Toast.success(message ?? 'HLS 时移缓存已保存')
      void mutate()
    } catch (failure) {
      Toast.error(relayErrorMessage(failure))
    } finally {
      setSaving(false)
    }
  }

  return (
    <section className={styles.card}>
      <div className={styles.sectionTitle}>
        <div className={styles.stack}>
          <Title heading={6} style={{ margin: 0 }}>
            HLS 时移缓存
          </Title>
          <Text type="tertiary" size="small">
            先把直播切片缓存在本地再吐出，用固定延迟换更稳的推流；延迟越大越抗网络抖动。
          </Text>
        </div>
        <Tag color={loaded && data.enabled ? 'green' : 'grey'} size="small">
          {!loaded ? (isLoading ? '加载中…' : '未加载') : data.enabled ? '已启用' : '未启用'}
        </Tag>
      </div>

      <dl className={styles.kv}>
        <dt className={styles.kvKey}>当前状态</dt>
        <dd className={styles.kvValue}>
          {loaded ? (data.enabled ? '启用' : '未启用') : isLoading ? '加载中…' : '—'}
        </dd>
        <dt className={styles.kvKey}>当前延迟</dt>
        <dd className={styles.kvValue}>
          {loaded ? `${data.latency_secs} 秒` : isLoading ? '加载中…' : '—'}
        </dd>
      </dl>

      {error ? (
        <Text type="danger" size="small">
          {relayErrorMessage(error)}
        </Text>
      ) : null}

      <div className={styles.toolbar}>
        <Switch
          checked={value?.enabled ?? false}
          onChange={(next) => setDraft({ enabled: next, latency: value?.latency ?? LATENCY_MIN })}
          disabled={!canEdit || !loaded}
          aria-label="启用 HLS 时移缓存"
        />
        <Text size="small">启用</Text>
        <InputNumber
          value={value?.latency ?? ''}
          disabled={!canEdit || !loaded}
          onChange={(next) => setDraft({ enabled: value?.enabled ?? false, latency: next })}
          style={{ width: 120 }}
          aria-label="时移延迟（秒）"
        />
        {latencyHint ? (
          <Text type="danger" size="small">
            {latencyHint}
          </Text>
        ) : (
          <Text type="tertiary" size="small">
            秒（{LATENCY_MIN}–{LATENCY_MAX}，整数）
          </Text>
        )}
        {canEdit ? (
          <Button
            theme="solid"
            loading={saving}
            disabled={!latencyValid}
            onClick={() => {
              void save()
            }}
          >
            保存时移缓存
          </Button>
        ) : null}
      </div>

      {canEdit ? null : (
        <Text type="tertiary" size="small">
          当前角色只能查看；修改需要「修改转播配置」权限。
        </Text>
      )}
    </section>
  )
}
