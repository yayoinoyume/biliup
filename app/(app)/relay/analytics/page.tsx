'use client'

/**
 * 推流统计与开播日历（`/relay/analytics`）。
 *
 * 数据来自 `/v1/relay/history/*` 五个端点：汇总 KPI、日历、可靠性、曲线与列表。
 * 所有历史请求都走 `useHistory` 里的并发闸（后端每节点只有两个统计查询槽位），
 * 条件请求命中 304 时保留上一份数据而不是清空。
 */

import { useCallback, useEffect, useMemo, useState } from 'react'
import { Banner, Button, DatePicker, Input, Select, Spin, Switch, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { IconHistogram } from '@douyinfe/semi-icons'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import { FormDialog } from '@/app/ui/shell'
import { useMe } from '@/app/lib/use-me'
import {
  isRelayApiError,
  relayHistoryClean,
  type RelayHistoryCleanReport,
  type RelayHistoryQuery,
} from '@/app/lib/relay'
import { formatBytes, formatDurationSec } from '@/app/lib/relay-format'
import relay from '@/app/ui/relay/relay.module.scss'
import Calendar from './Calendar'
import Events from './Events'
import Hours from './Hours'
import Kpi from './Kpi'
import Nodes from './Nodes'
import Reliability from './Reliability'
import Strip from './Strip'
import Audience from './Audience'
import Quality from './Quality'
import {
  useHistory,
  useHistoryEvents,
  useHistorySessions,
  windowQuery,
  type HistoryBucket,
  type HistoryPlatform,
  type HistoryWindow,
} from './useHistory'
import styles from './analytics.module.scss'

const { Text } = Typography

const DAY = 86400

type Range = '7d' | '30d' | '90d' | 'custom'

const RANGE_OPTIONS = [
  { label: '最近 7 天', value: '7d' },
  { label: '最近 30 天', value: '30d' },
  { label: '最近 90 天', value: '90d' },
  { label: '自定义', value: 'custom' },
]

const PLATFORM_OPTIONS = [
  { label: '全部平台', value: 'all' },
  { label: 'YouTube', value: 'YT' },
  { label: 'Twitch', value: 'TW' },
  { label: 'Niconico', value: 'NC' },
]

const BUCKETS: Record<Exclude<Range, 'custom'>, HistoryBucket> = {
  '7d': 'hour',
  '30d': '6h',
  '90d': 'day',
}

const presetSecs = (range: Range) =>
  range === '7d' ? 7 * DAY : range === '30d' ? 30 * DAY : 90 * DAY

const bucketSecs = (bucket: HistoryBucket) => (bucket === 'hour' ? 3600 : bucket === '6h' ? 21600 : DAY)

function cleanMessage(report: RelayHistoryCleanReport): string {
  const nodes = report.nodes.map((node) => `${node.node_id} ${node.rows} 行`).join('，')
  const failed = report.failed.map((item) => `${item.node_id}（${item.reason}）`).join('，')
  return `共清理 ${report.rows} 行${nodes ? `：${nodes}` : ''}${failed ? `；未清理：${failed}` : ''}`
}

export default function RelayAnalyticsPage() {
  const { can } = useMe()
  const [range, setRange] = useState<Range>('7d')
  const [anchor, setAnchor] = useState(() => Math.floor(Date.now() / 1000))
  const [custom, setCustom] = useState<{ from: number; to: number } | null>(null)
  const [platform, setPlatform] = useState<'all' | HistoryPlatform>('all')
  const [channel, setChannel] = useState('')
  const [autoRefresh, setAutoRefresh] = useState(false)
  const [maxPoints, setMaxPoints] = useState(360)
  const [dayFilter, setDayFilter] = useState<number | null>(null)

  const [cleanOpen, setCleanOpen] = useState(false)
  const [cleanError, setCleanError] = useState<unknown>(null)

  const timeWindow = useMemo<HistoryWindow>(() => {
    const useCustom = range === 'custom' && custom !== null
    const to = useCustom && custom ? custom.to : anchor
    const from = useCustom && custom ? custom.from : anchor - presetSecs(range)
    return {
      from,
      to,
      bucket: range === 'custom' ? 'day' : BUCKETS[range],
      platform: platform === 'all' ? undefined : platform,
      channel: channel.trim() || undefined,
    }
  }, [range, custom, anchor, platform, channel])

  const history = useHistory(timeWindow, maxPoints)

  const sessionsQuery = useMemo<RelayHistoryQuery>(
    () => ({
      ...windowQuery(timeWindow),
      from: dayFilter ?? timeWindow.from,
      to: dayFilter ? dayFilter + DAY : timeWindow.to,
      limit: 50,
    }),
    [timeWindow, dayFilter],
  )
  const eventsQuery = useMemo<RelayHistoryQuery>(
    () => ({
      ...windowQuery(timeWindow),
      from: dayFilter ?? timeWindow.from,
      to: dayFilter ? dayFilter + DAY : timeWindow.to,
      limit: 50,
    }),
    [timeWindow, dayFilter],
  )
  const sessions = useHistorySessions(sessionsQuery)
  const events = useHistoryEvents(eventsQuery)

  const { reload: reloadHistory } = history
  const { reload: reloadSessions } = sessions
  const { reload: reloadEvents } = events

  /**
   * 刷新：换一个新的 `to` 就会让所有查询 key 变化、自动重发；
   * 同一秒内（key 不变）才手动重发，避免一次点击打出两轮请求。
   */
  const reloadAll = useCallback(() => {
    const now = Math.floor(Date.now() / 1000)
    if (now === anchor) {
      reloadHistory()
      reloadSessions()
      reloadEvents()
      return
    }
    setAnchor(now)
  }, [anchor, reloadHistory, reloadSessions, reloadEvents])

  useEffect(() => {
    if (!autoRefresh) return
    const timer = setInterval(() => {
      setAnchor(Math.floor(Date.now() / 1000))
    }, 60000)
    return () => clearInterval(timer)
  }, [autoRefresh])

  const changeRange = (value: string) => {
    if (value !== '7d' && value !== '30d' && value !== '90d' && value !== 'custom') return
    setRange(value)
    setAnchor(Math.floor(Date.now() / 1000))
  }

  const statsTooLarge = isRelayApiError(history.statsError) && history.statsError.status === 413
  const busy = isRelayApiError(history.summaryError) && history.summaryError.status === 429

  const runClean = async () => {
    setCleanError(null)
    try {
      const report = await relayHistoryClean()
      Toast.success(cleanMessage(report))
      setCleanOpen(false)
      reloadAll()
    } catch (error) {
      setCleanError(error)
    }
  }

  const status = history.status
  const summary = history.summary
  const stats = history.stats
  const canClean = can('relay.control')

  return (
    <div className={relay.content}>
      <PageHeader
        title="推流统计"
        description="时长、可用率、失败原因与在线曲线"
        icon={<IconHistogram />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
        actions={
          <div className={relay.toolbar}>
            <span className={styles.autoSwitch}>
              <Text type="tertiary" size="small">
                自动刷新
              </Text>
              <Switch size="small" checked={autoRefresh} onChange={setAutoRefresh} />
            </span>
            <Button theme="light" onClick={reloadAll} loading={history.loading}>
              刷新
            </Button>
          </div>
        }
      />

      <PermissionGate perm="relay.view">
        <section className={relay.card}>
          <div className={relay.toolbar}>
            <Select
              value={range}
              optionList={RANGE_OPTIONS}
              onChange={(value) => changeRange(String(value))}
              style={{ width: 150 }}
            />
            {range === 'custom' ? (
              <DatePicker
                type="dateRange"
                style={{ width: 260 }}
                onChange={(value) => {
                  if (!Array.isArray(value) || value.length !== 2) return
                  const [start, end] = value
                  if (!(start instanceof Date) || !(end instanceof Date)) return
                  setCustom({
                    from: Math.floor(start.getTime() / 1000),
                    to: Math.floor(end.getTime() / 1000),
                  })
                }}
              />
            ) : null}
            <Select
              value={platform}
              optionList={PLATFORM_OPTIONS}
              onChange={(value) => {
                const next = String(value)
                setPlatform(next === 'YT' || next === 'TW' || next === 'NC' ? next : 'all')
              }}
              style={{ width: 140 }}
            />
            <Input
              value={channel}
              onChange={setChannel}
              placeholder="频道名或 id"
              style={{ width: 200 }}
              showClear
            />
            {dayFilter !== null ? (
              <Tag
                size="small"
                closable
                onClose={() => setDayFilter(null)}
              >
                只看 {new Date(dayFilter * 1000).toLocaleDateString('zh-CN')}
              </Tag>
            ) : null}
            <span className={relay.toolbarGrow} />
            {status ? (
              <span className={styles.statusLine}>
                <Tag size="small" color={status.enabled ? 'green' : 'grey'}>
                  {status.enabled ? '记录中' : '未记录'}
                </Tag>
                <Text type="tertiary" size="small">
                  {formatBytes(status.db_bytes)} · 保留 {status.retention.raw_days} 天 · 场次{' '}
                  {status.rows.sessions ?? '—'}
                </Text>
              </span>
            ) : null}
          </div>
        </section>

        {history.summaryError && !busy ? (
          <RelayError error={history.summaryError} onRetry={reloadAll} title="汇总加载失败" />
        ) : null}
        {history.statusError ? (
          <RelayError error={history.statusError} onRetry={reloadHistory} title="统计库状态加载失败" />
        ) : null}
        {busy ? (
          <Banner
            type="warning"
            title="统计查询繁忙"
            description="后端每个节点只允许两个并发统计查询，稍等片刻再刷新。"
            closeIcon={null}
          />
        ) : null}
        {statsTooLarge ? (
          <Banner
            type="warning"
            title="统计结果过大"
            description={
              <div className={styles.bannerBody}>
                <Text>后端合并后的响应超过 1MiB。降低采样点或缩小时间范围再看。</Text>
                <Button
                  size="small"
                  theme="light"
                  onClick={() => setMaxPoints((value) => Math.max(60, Math.floor(value / 2)))}
                >
                  采样点减半重试
                </Button>
              </div>
            }
            closeIcon={null}
          />
        ) : null}

        {summary ? (
          <section className={relay.card}>
            <div className={relay.sectionTitle}>
              <Text strong>汇总指标</Text>
              <Text type="tertiary" size="small">
                与上一个等长周期对比
              </Text>
            </div>
            <Kpi kpis={summary.kpis} prev={summary.prev_kpis} />
          </section>
        ) : history.loading ? (
          <section className={relay.card}>
            <Spin size="large" />
          </section>
        ) : null}

        {summary ? (
          <>
            <div className={relay.grid}>
              <section className={relay.card}>
                <div className={relay.sectionTitle}>
                  <Text strong>在线状态</Text>
                  <Text type="tertiary" size="small">
                    {summary.buckets.length} 个时间桶
                  </Text>
                </div>
                <Strip buckets={summary.buckets} bucketSecs={bucketSecs(timeWindow.bucket)} />
              </section>
              <section className={relay.card}>
                <div className={relay.sectionTitle}>
                  <Text strong>推流时长</Text>
                  <Text type="tertiary" size="small">
                    合计 {formatDurationSec(summary.kpis.live_secs)}
                  </Text>
                </div>
                <Hours buckets={summary.buckets} />
              </section>
            </div>

            {stats ? (
              <>
                <section className={relay.card}>
                  <div className={relay.sectionTitle}>
                    <Text strong>观众数</Text>
                    <Text type="tertiary" size="small">
                      清洗后 {stats.resolution === '1m' ? '按分钟' : '原始采样'} · {stats.ts.length} 点
                    </Text>
                  </div>
                  <Audience stats={stats} />
                </section>
                <section className={relay.card}>
                  <div className={relay.sectionTitle}>
                    <Text strong>推流质量</Text>
                    <Text type="tertiary" size="small">
                      码率与帧率，含 min / max 包络
                    </Text>
                  </div>
                  <Quality stats={stats} />
                </section>
              </>
            ) : null}

            <div className={relay.grid}>
              <section className={relay.card}>
                <div className={relay.sectionTitle}>
                  <Text strong>可靠性</Text>
                </div>
                <Reliability reliability={summary.reliability} />
              </section>
              <section className={relay.card}>
                <div className={relay.sectionTitle}>
                  <Text strong>节点覆盖</Text>
                </div>
                <Nodes summary={summary} />
              </section>
            </div>

            <section className={relay.card}>
              <div className={relay.sectionTitle}>
                <Text strong>开播日历</Text>
                <Text type="tertiary" size="small">
                  点日期把下面的场次收窄到当天
                </Text>
              </div>
              <Calendar calendar={summary.calendar} selected={dayFilter} onSelect={setDayFilter} />
            </section>
          </>
        ) : null}

        <Events sessions={sessions} events={events} />

        {canClean ? (
          <section className={relay.card}>
            <div className={relay.sectionTitle}>
              <Text strong>危险操作</Text>
            </div>
            <div className={styles.dangerRow}>
              <Text type="tertiary" size="small">
                清空本节点已记录的场次、事件与曲线行；集群里别的节点保留自己的行。
              </Text>
              <Button type="danger" theme="light" onClick={() => setCleanOpen(true)}>
                清理历史统计
              </Button>
            </div>
          </section>
        ) : null}

        <FormDialog
          title="清理历史统计"
          visible={cleanOpen}
          size="sm"
          okText="清理"
          okType="danger"
          onOk={runClean}
          onCancel={() => setCleanOpen(false)}
          afterClose={() => setCleanOpen(false)}
        >
          <div className={relay.stack}>
            <Text>
              这会删掉本节点统计库里「全部」已记录的行，删完无法恢复；正在转播的节点会被后端拒绝。
            </Text>
            <Text type="tertiary" size="small">
              当前保留期：原始采样 {status?.retention.raw_days ?? '—'} 天，汇总{' '}
              {status?.retention.rollup_days ?? '—'} 天，事件 {status?.retention.event_days ?? '—'} 天。
              后端清理接口不接受保留天数参数；只想保留最近 N 天的话，请到设置页把保留天数调低，
              写入线程会按天自动淘汰。
            </Text>
            {cleanError ? <RelayError error={cleanError} title="清理失败" /> : null}
          </div>
        </FormDialog>
      </PermissionGate>
    </div>
  )
}
