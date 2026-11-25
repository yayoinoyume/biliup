'use client'

/**
 * 首次引导（`/relay/setup`）：Phase 3 底座页。
 *
 * 只读地展示 `/v1/relay/setup-status`，确认「是否需要引导」这个判据能取到；
 * 完整的四步向导（扫码登录、Holodex 收藏、分区选择）由 P3-02 接管。
 */

import { Tag, Typography } from '@douyinfe/semi-ui'
import { IconArticle } from '@douyinfe/semi-icons'
import useSWR from 'swr'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import { relaySetupStatus } from '@/app/lib/relay'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

export default function RelaySetupPage() {
  const setup = useSWR('relay:setup-status', relaySetupStatus, { revalidateOnFocus: false })
  const data = setup.data

  return (
    <div className={styles.content}>
      <PageHeader
        title="转播向导"
        description="首次安装的检查项与扫码登录入口"
        icon={<IconArticle />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
      />

      <PermissionGate perm="relay.config">
        {setup.error ? (
          <RelayError
            error={setup.error}
            onRetry={() => {
              void setup.mutate()
            }}
          />
        ) : null}

        <section className={styles.card}>
          <div className={styles.sectionTitle}>
            <Title heading={6} style={{ margin: 0 }}>
              安装检查
            </Title>
            {data ? (
              <Tag size="small" color={data.needs_setup ? 'orange' : 'green'}>
                {data.needs_setup ? '需要引导' : '已就绪'}
              </Tag>
            ) : null}
          </div>
          <dl className={styles.kv}>
            <dt className={styles.kvKey}>缺失文件</dt>
            <dd className={styles.kvValue}>
              {data?.missing_files.length ? data.missing_files.join('、') : '无'}
            </dd>
            <dt className={styles.kvKey}>引导命令</dt>
            <dd className={`${styles.kvValue} ${styles.mono}`}>{data?.setup_command || '—'}</dd>
            <dt className={styles.kvKey}>存储错误</dt>
            <dd className={styles.kvValue}>{data?.storage_error ?? '无'}</dd>
          </dl>
          <Text type="tertiary" size="small">
            向导正文由 P3-02 接管；本页现在只验证只读检查链路。
          </Text>
        </section>
      </PermissionGate>
    </div>
  )
}
