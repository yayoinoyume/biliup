'use client'

/**
 * 推流统计（`/relay/analytics`）：Phase 3 底座页。
 *
 * 只读地展示 `/v1/relay/history/status`（ETag 端点）的健康状况，
 * 证明条件请求链路可用；KPI、日历与曲线由 P3-05 接管。
 */

import { Tag, Typography } from '@douyinfe/semi-ui'
import { IconHistogram } from '@douyinfe/semi-icons'
import useSWR from 'swr'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import { relayHistoryStatus } from '@/app/lib/relay'
import { formatBytes, formatUnixSec } from '@/app/lib/relay-format'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

export default function RelayAnalyticsPage() {
  const history = useSWR('relay:history-status', () => relayHistoryStatus(), {
    revalidateOnFocus: false,
  })
  const status = history.data?.notModified ? null : history.data?.data ?? null

  return (
    <div className={styles.content}>
      <PageHeader
        title="推流统计"
        description="时长、可用率、失败原因与在线曲线"
        icon={<IconHistogram />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
      />

      <PermissionGate perm="relay.view">
        {history.error ? (
          <RelayError
            error={history.error}
            onRetry={() => {
              void history.mutate()
            }}
          />
        ) : null}

        <section className={styles.card}>
          <div className={styles.sectionTitle}>
            <Title heading={6} style={{ margin: 0 }}>
              统计库
            </Title>
            {status ? (
              <Tag size="small" color={status.enabled ? 'green' : 'grey'}>
                {status.enabled ? '记录中' : '未记录'}
              </Tag>
            ) : null}
          </div>
          <div className={styles.statGrid}>
            <div className={styles.stat}>
              <span className={styles.statLabel}>库大小</span>
              <span className={styles.statValue}>{formatBytes(status?.db_bytes)}</span>
            </div>
            <div className={styles.stat}>
              <span className={styles.statLabel}>场次</span>
              <span className={styles.statValue}>{status?.rows.sessions ?? '—'}</span>
            </div>
            <div className={styles.stat}>
              <span className={styles.statLabel}>事件</span>
              <span className={styles.statValue}>{status?.rows.events ?? '—'}</span>
            </div>
            <div className={styles.stat}>
              <span className={styles.statLabel}>最早记录</span>
              <span className={styles.statValue}>{formatUnixSec(status?.oldest_at)}</span>
            </div>
          </div>
          {status?.restart_required ? (
            <Text type="warning" size="small">
              统计记录开关已改，重启进程后生效。
            </Text>
          ) : (
            <Text type="tertiary" size="small">
              写入状态：{status?.writer.state ?? '—'}；KPI 与曲线由 P3-05 接管。
            </Text>
          )}
        </section>
      </PermissionGate>
    </div>
  )
}
