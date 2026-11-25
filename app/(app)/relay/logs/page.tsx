'use client'

/**
 * 实时日志（`/relay/logs`）：Phase 3 底座页。
 *
 * 先只读地拉一次 `/v1/relay/logs` 的内存环形日志，证明日志端点可用；
 * 跟随滚动的实时视图与筛选由 P3-05 接管。
 */

import { Tag, Typography } from '@douyinfe/semi-ui'
import { IconTerminal } from '@douyinfe/semi-icons'
import useSWR from 'swr'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import { relayLogs } from '@/app/lib/relay'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

/** 只展示末尾若干行，避免首屏被几万行日志拖死。 */
const TAIL_LINES = 40

export default function RelayLogsPage() {
  const logs = useSWR('relay:logs', relayLogs, { revalidateOnFocus: false })
  const lines = logs.data?.logs ? logs.data.logs.split('\n') : []
  const tail = lines.slice(-TAIL_LINES)

  return (
    <div className={styles.content}>
      <PageHeader
        title="实时日志"
        description="转播底座的内存环形日志"
        icon={<IconTerminal />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
      />

      <PermissionGate perm="relay.view">
        {logs.error ? (
          <RelayError
            error={logs.error}
            onRetry={() => {
              void logs.mutate()
            }}
          />
        ) : null}

        <section className={styles.card}>
          <div className={styles.sectionTitle}>
            <Title heading={6} style={{ margin: 0 }}>
              最近 {TAIL_LINES} 行
            </Title>
            <Tag size="small">{lines.length} 行</Tag>
          </div>
          {tail.length > 0 ? (
            <pre className={styles.mono} style={{ margin: 0, whiteSpace: 'pre-wrap' }}>
              {tail.join('\n')}
            </pre>
          ) : (
            <Text type="tertiary" size="small">
              {logs.isLoading ? '读取中…' : '暂时没有日志输出。'}
            </Text>
          )}
        </section>
      </PermissionGate>
    </div>
  )
}
