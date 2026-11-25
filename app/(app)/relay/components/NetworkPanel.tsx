'use client'

import { Tag, Typography } from '@douyinfe/semi-ui'
import useSWR from 'swr'
import { relayDepsStatus, relayNetworkStatus } from '@/app/lib/relay'
import { formatDurationSec } from '@/app/lib/relay-format'
import RelayError from '@/app/ui/relay/RelayError'
import styles from '@/app/ui/relay/relay.module.scss'
import { RELAY_DEPS_KEY, RELAY_NETWORK_KEY } from './keys'
import { StatusDot } from './ui'

const { Text, Title } = Typography

function kbps(value: number | null | undefined): string {
  return typeof value === 'number' && Number.isFinite(value) ? value.toFixed(0) + ' kbps' : '—'
}

function fps(value: number | null | undefined): string {
  return typeof value === 'number' && Number.isFinite(value) ? value.toFixed(1) : '—'
}

function seconds(value: number | null | undefined): string {
  return formatDurationSec(value)
}

export default function NetworkPanel() {
  const network = useSWR(RELAY_NETWORK_KEY, relayNetworkStatus, {
    refreshInterval: 5000,
    revalidateOnFocus: false,
  })
  const deps = useSWR(RELAY_DEPS_KEY, relayDepsStatus, { revalidateOnFocus: false })
  const data = network.data
  const cacheDelay =
    typeof data?.stream_cache_time_secs === 'number' && typeof data.stream_time_secs === 'number'
      ? Math.max(0, data.stream_cache_time_secs - data.stream_time_secs)
      : null

  return (
    <section className={styles.card}>
      <div className={styles.sectionTitle}>
        <Title heading={6} style={{ margin: 0 }}>
          网络速率与推流
        </Title>
        <div className={styles.toolbar}>
          <Tag size="small" color={data?.ffmpeg_running ? 'green' : 'grey'}>
            {data?.ffmpeg_running ? '推流中' : '未推流'}
          </Tag>
          <Tag size="small" color={data?.hls_cache_active ? 'green' : 'grey'}>
            {data?.hls_cache_active ? 'HLS 缓存运行' : 'HLS 缓存空闲'}
          </Tag>
          <Tag size="small" color={deps.data ? (deps.data.ready ? 'green' : 'orange') : 'grey'}>
            {deps.data ? (deps.data.ready ? '依赖就绪' : '依赖待补') : '依赖检查中'}
          </Tag>
        </div>
      </div>

      {network.error ? (
        <RelayError
          error={network.error}
          title="推流状态读取失败"
          onRetry={() => {
            void network.mutate()
          }}
        />
      ) : null}
      {deps.error ? (
        <RelayError
          error={deps.error}
          title="推流依赖读取失败"
          onRetry={() => {
            void deps.mutate()
          }}
        />
      ) : null}

      <div className={styles.statGrid}>
        <div className={styles.stat}>
          <span className={styles.statLabel}>上行码率</span>
          <span className={styles.statValue}>{kbps(data?.stream_bitrate_kbps)}</span>
        </div>
        <div className={styles.stat}>
          <span className={styles.statLabel}>缓存码率</span>
          <span className={styles.statValue}>{kbps(data?.stream_cache_bitrate_kbps)}</span>
        </div>
        <div className={styles.stat}>
          <span className={styles.statLabel}>帧率</span>
          <span className={styles.statValue}>{fps(data?.stream_fps)}</span>
        </div>
        <div className={styles.stat}>
          <span className={styles.statLabel}>已推时长</span>
          <span className={styles.statValue}>{seconds(data?.stream_time_secs)}</span>
        </div>
        <div className={styles.stat}>
          <span className={styles.statLabel}>缓存进度</span>
          <span className={styles.statValue}>{seconds(data?.stream_cache_time_secs)}</span>
        </div>
        <div className={styles.stat}>
          <span className={styles.statLabel}>缓存领先</span>
          <span className={styles.statValue}>{seconds(cacheDelay)}</span>
        </div>
      </div>

      <dl className={styles.kv} style={{ marginTop: 12 }}>
        <dt className={styles.kvKey}>ffmpeg</dt>
        <dd className={styles.kvValue}>
          <StatusDot online={deps.data?.ffmpeg === true} />{' '}
          {deps.data ? (deps.data.ffmpeg ? '已安装' : '未检测到') : '读取中…'}
        </dd>
        <dt className={styles.kvKey}>yt-dlp</dt>
        <dd className={styles.kvValue}>
          <StatusDot online={deps.data?.yt_dlp === true} />{' '}
          {deps.data ? (deps.data.yt_dlp ? '已安装' : '未检测到') : '读取中…'}
        </dd>
        <dt className={styles.kvKey}>streamlink</dt>
        <dd className={styles.kvValue}>
          <StatusDot online={deps.data?.streamlink === true} />{' '}
          {deps.data ? (deps.data.streamlink ? '已安装' : '未检测到') : '读取中…'}
        </dd>
      </dl>
      <Text type="tertiary" size="small">
        指标每 5 秒刷新一次；yt-dlp 与 streamlink 至少有其一即可开播。
      </Text>
    </section>
  )
}
