'use client'

/**
 * 单个场次的详情弹窗：完整字段 + 这一场的 `/history/stats` 曲线。
 * 曲线请求只带这一个 `session`，时间窗就是这一场的起止，避免落到默认的「最近一天」。
 *
 * 正文拆成按 `session.id` 挂载的子组件：换一场就重挂载，加载态从初始值开始，
 * 不需要在 effect 里同步重置 state。
 */

import { useEffect, useState } from 'react'
import { Spin, Tag, Typography } from '@douyinfe/semi-ui'
import { FormDialog } from '@/app/ui/shell'
import {
  relayHistoryStats,
  type RelayHistorySessionItem,
  type RelayHistoryStats,
} from '@/app/lib/relay'
import { formatDurationSec, formatUnixSec } from '@/app/lib/relay-format'
import RelayError from '@/app/ui/relay/RelayError'
import relay from '@/app/ui/relay/relay.module.scss'
import { historySlot } from './useHistory'
import Audience from './Audience'
import Quality from './Quality'
import styles from './analytics.module.scss'

const { Text } = Typography

interface SessionBodyProps {
  session: RelayHistorySessionItem
}

interface CurveState {
  stats: RelayHistoryStats | null
  error: unknown
  loading: boolean
}

function SessionBody({ session }: SessionBodyProps) {
  const [curve, setCurve] = useState<CurveState>({ stats: null, error: null, loading: true })

  useEffect(() => {
    let cancelled = false
    void historySlot(() =>
      relayHistoryStats({
        session: session.id,
        from: session.started_at,
        to: session.ended_at ?? Math.floor(Date.now() / 1000),
      }),
    ).then(
      (result) => {
        if (cancelled) return
        setCurve(
          result.notModified
            ? { stats: null, error: null, loading: false }
            : { stats: result.data, error: null, loading: false },
        )
      },
      (error: unknown) => {
        if (cancelled) return
        setCurve({ stats: null, error, loading: false })
      },
    )
    return () => {
      cancelled = true
    }
  }, [session])

  return (
    <div className={relay.stack}>
      <div className={relay.sectionTitle}>
        <Text strong>{session.title || '（无标题）'}</Text>
        <span className={styles.tagRow}>
          <Tag size="small">{session.platform}</Tag>
          <Tag size="small" color={session.failures > 0 ? 'red' : 'green'}>
            {session.failures > 0 ? `${session.failures} 次失败` : '无失败'}
          </Tag>
        </span>
      </div>

      <dl className={relay.kv}>
        <dt className={relay.kvKey}>场次 id</dt>
        <dd className={`${relay.kvValue} ${relay.mono}`}>{session.id}</dd>
        <dt className={relay.kvKey}>节点</dt>
        <dd className={relay.kvValue}>{session.node}</dd>
        <dt className={relay.kvKey}>频道</dt>
        <dd className={relay.kvValue}>
          {session.channel_name}（{session.channel_id}）
        </dd>
        <dt className={relay.kvKey}>开始</dt>
        <dd className={relay.kvValue}>{formatUnixSec(session.started_at)}</dd>
        <dt className={relay.kvKey}>结束</dt>
        <dd className={relay.kvValue}>{formatUnixSec(session.ended_at)}</dd>
        <dt className={relay.kvKey}>时长</dt>
        <dd className={relay.kvValue}>{formatDurationSec(session.duration_secs)}</dd>
        <dt className={relay.kvKey}>结束原因</dt>
        <dd className={relay.kvValue}>{session.end_reason ?? '—'}</dd>
        <dt className={relay.kvKey}>运行 / 重启 / 卡死</dt>
        <dd className={relay.kvValue}>
          {session.runs} / {session.restarts} / {session.stuck}
        </dd>
        <dt className={relay.kvKey}>码率中位数</dt>
        <dd className={relay.kvValue}>
          {session.median_bitrate_kbps === null ? '—' : `${session.median_bitrate_kbps} kbps`}
        </dd>
        {session.end_detail ? (
          <>
            <dt className={relay.kvKey}>结束细节</dt>
            <dd className={relay.kvValue}>{session.end_detail}</dd>
          </>
        ) : null}
      </dl>

      {session.bili ? (
        <dl className={relay.kv}>
          <dt className={relay.kvKey}>B 站已看</dt>
          <dd className={relay.kvValue}>{session.bili.watched ?? '—'}</dd>
          <dt className={relay.kvKey}>在线峰值</dt>
          <dd className={relay.kvValue}>{session.bili.online_peak ?? '—'}</dd>
          <dt className={relay.kvKey}>最高在线</dt>
          <dd className={relay.kvValue}>{session.bili.max_online ?? '—'}</dd>
          <dt className={relay.kvKey}>弹幕 / 涨粉</dt>
          <dd className={relay.kvValue}>
            {session.bili.danmu ?? '—'} / {session.bili.add_fans ?? '—'}
          </dd>
        </dl>
      ) : null}

      {session.gaps.length > 0 ? (
        <div>
          <Text strong size="small">
            断档
          </Text>
          <ul className={styles.reasons}>
            {session.gaps.map(([start, end, reason]) => (
              <li key={`${start}-${end}`}>
                <span>
                  {formatUnixSec(start)} → {formatUnixSec(end)}
                </span>
                <span className={relay.muted}>{reason}</span>
              </li>
            ))}
          </ul>
        </div>
      ) : null}

      <div>
        <Text strong size="small">
          这一场的曲线
        </Text>
        {curve.error ? <RelayError error={curve.error} title="曲线加载失败" /> : null}
        {curve.loading ? <Spin size="small" /> : null}
        {curve.stats ? (
          <div className={relay.stack}>
            <Audience stats={curve.stats} />
            <Quality stats={curve.stats} />
          </div>
        ) : !curve.loading && !curve.error ? (
          <p className={relay.muted}>这一场没有采样。</p>
        ) : null}
      </div>
    </div>
  )
}

export interface SessionDetailProps {
  session: RelayHistorySessionItem | null
  onClose: () => void
}

export default function SessionDetail({ session, onClose }: SessionDetailProps) {
  return (
    <FormDialog
      title={session ? `场次详情 · ${session.channel_name || session.channel_id}` : '场次详情'}
      visible={session !== null}
      size="lg"
      footer={null}
      onCancel={onClose}
      afterClose={onClose}
    >
      {session ? <SessionBody key={session.id} session={session} /> : null}
    </FormDialog>
  )
}
