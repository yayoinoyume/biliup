'use client'

/**
 * 场次与事件两块列表，都按后端游标翻页：`next_cursor` 为空才是真的没有更多。
 * 点场次行打开详情弹窗，弹窗里再单独拉这一场的曲线。
 */

import { useState } from 'react'
import { Button, Spin, Tag, Typography } from '@douyinfe/semi-ui'
import type {
  RelayHistoryEventItem,
  RelayHistorySessionItem,
} from '@/app/lib/relay'
import { formatDurationSec, formatUnixSec } from '@/app/lib/relay-format'
import RelayError from '@/app/ui/relay/RelayError'
import relay from '@/app/ui/relay/relay.module.scss'
import type { PagedHistory } from './useHistory'
import SessionDetail from './SessionDetail'
import styles from './analytics.module.scss'

const { Text } = Typography

const SEVERITY_COLOR: Record<string, 'red' | 'orange' | 'blue' | 'grey'> = {
  error: 'red',
  warn: 'orange',
  warning: 'orange',
  info: 'blue',
  debug: 'grey',
}

const severityColor = (severity: string) => SEVERITY_COLOR[severity.toLowerCase()] ?? 'grey'

export interface EventsProps {
  sessions: PagedHistory<{ items: RelayHistorySessionItem[]; next_cursor: string | null }>
  events: PagedHistory<{ items: RelayHistoryEventItem[]; next_cursor: string | null }>
}

export default function Events({ sessions, events }: EventsProps) {
  const [selected, setSelected] = useState<RelayHistorySessionItem | null>(null)

  return (
    <div className={styles.listPair}>
      <section className={relay.card}>
        <div className={relay.sectionTitle}>
          <Text strong>场次</Text>
          <Text type="tertiary" size="small">
            共 {sessions.items.length} 场
          </Text>
        </div>
        {sessions.error ? (
          <RelayError error={sessions.error} onRetry={sessions.reload} title="场次加载失败" />
        ) : null}
        <div className={relay.tableWrap}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th scope="col">开始</th>
                <th scope="col">频道</th>
                <th scope="col">平台</th>
                <th scope="col">时长</th>
                <th scope="col">失败</th>
                <th scope="col">结束原因</th>
              </tr>
            </thead>
            <tbody>
              {sessions.items.map((session) => (
                <tr key={session.id}>
                  <td>{formatUnixSec(session.started_at)}</td>
                  <td>
                    <button
                      type="button"
                      className={styles.linkButton}
                      onClick={() => setSelected(session)}
                    >
                      {session.channel_name || session.channel_id}
                    </button>
                    {session.title ? (
                      <div className={relay.muted}>{session.title}</div>
                    ) : null}
                  </td>
                  <td>{session.platform}</td>
                  <td>{formatDurationSec(session.duration_secs)}</td>
                  <td>
                    {session.failures}
                    {session.restarts > 0 ? ` / 重启 ${session.restarts}` : ''}
                  </td>
                  <td>{session.end_reason ?? '—'}</td>
                </tr>
              ))}
              {sessions.items.length === 0 && !sessions.loading ? (
                <tr>
                  <td colSpan={6}>
                    <span className={relay.muted}>这段区间没有场次。</span>
                  </td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
        <div className={styles.listFooter}>
          {sessions.loading ? <Spin size="small" /> : null}
          {sessions.nextCursor ? (
            <Button
              size="small"
              theme="light"
              loading={sessions.loadingMore}
              onClick={sessions.loadMore}
            >
              加载更多场次
            </Button>
          ) : (
            <Text type="tertiary" size="small">
              {sessions.items.length > 0 ? '已经到底了' : ''}
            </Text>
          )}
        </div>
      </section>

      <section className={relay.card}>
        <div className={relay.sectionTitle}>
          <Text strong>事件</Text>
          <Text type="tertiary" size="small">
            共 {events.items.length} 条
          </Text>
        </div>
        {events.error ? (
          <RelayError error={events.error} onRetry={events.reload} title="事件加载失败" />
        ) : null}
        <div className={relay.tableWrap}>
          <table className={styles.table}>
            <thead>
              <tr>
                <th scope="col">时间</th>
                <th scope="col">级别</th>
                <th scope="col">类型</th>
                <th scope="col">内容</th>
              </tr>
            </thead>
            <tbody>
              {events.items.map((event) => (
                <tr key={event.id}>
                  <td>{formatUnixSec(event.at)}</td>
                  <td>
                    <Tag size="small" color={severityColor(event.severity)}>
                      {event.severity}
                    </Tag>
                  </td>
                  <td className={relay.mono}>{event.kind}</td>
                  <td>
                    {event.title}
                    {event.detail ? <div className={relay.muted}>{event.detail}</div> : null}
                    {event.notify ? (
                      <div className={relay.muted}>
                        通知 {event.notify.ok ? '已送达' : '未送达'}
                        {event.notify.code !== null ? `（HTTP ${event.notify.code}）` : ''}
                      </div>
                    ) : null}
                  </td>
                </tr>
              ))}
              {events.items.length === 0 && !events.loading ? (
                <tr>
                  <td colSpan={4}>
                    <span className={relay.muted}>这段区间没有事件。</span>
                  </td>
                </tr>
              ) : null}
            </tbody>
          </table>
        </div>
        <div className={styles.listFooter}>
          {events.loading ? <Spin size="small" /> : null}
          {events.nextCursor ? (
            <Button
              size="small"
              theme="light"
              loading={events.loadingMore}
              onClick={events.loadMore}
            >
              加载更多事件
            </Button>
          ) : (
            <Text type="tertiary" size="small">
              {events.items.length > 0 ? '已经到底了' : ''}
            </Text>
          )}
        </div>
      </section>

      <SessionDetail session={selected} onClose={() => setSelected(null)} />
    </div>
  )
}
