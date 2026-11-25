'use client'

/**
 * 频道与分区（`/relay/manage`）：Phase 3 底座页。
 *
 * 只读地列出本机频道与分区文档的规模，确认 `/v1/relay/manage/*` 两个读端可用；
 * 增删改表单由 P3-02 接管。
 */

import { Tag, Typography } from '@douyinfe/semi-ui'
import { IconServerStroked } from '@douyinfe/semi-icons'
import useSWR from 'swr'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import { relayManageAreas, relayManageChannels } from '@/app/lib/relay'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

export default function RelayManagePage() {
  const channels = useSWR('relay:manage-channels', relayManageChannels, { revalidateOnFocus: false })
  const areas = useSWR('relay:manage-areas', relayManageAreas, { revalidateOnFocus: false })

  const channelRows = channels.data?.channels ?? []
  const areaRows = areas.data?.areas ?? []

  return (
    <div className={styles.content}>
      <PageHeader
        title="频道与分区"
        description="监听名单、分区规则与禁用关键词"
        icon={<IconServerStroked />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
      />

      <PermissionGate perm="relay.view">
        {channels.error ? (
          <RelayError
            error={channels.error}
            title="频道名单加载失败"
            onRetry={() => {
              void channels.mutate()
            }}
          />
        ) : null}
        {areas.error ? (
          <RelayError
            error={areas.error}
            title="分区规则加载失败"
            onRetry={() => {
              void areas.mutate()
            }}
          />
        ) : null}

        <div className={styles.grid}>
          <section className={styles.card}>
            <div className={styles.sectionTitle}>
              <Title heading={6} style={{ margin: 0 }}>
                频道
              </Title>
              <Tag size="small">{channelRows.length} 个</Tag>
            </div>
            <div className={styles.stack}>
              {channelRows.slice(0, 8).map((channel) => (
                <div key={channel.name} className={styles.kv}>
                  <span className={styles.kvKey}>{channel.name}</span>
                  <span className={styles.kvValue}>
                    {Object.entries(channel.platforms)
                      .map(([platform, id]) => `${platform}: ${id}`)
                      .join('，') || '未绑定平台'}
                  </span>
                </div>
              ))}
              {channelRows.length > 8 ? (
                <Text type="tertiary" size="small">
                  仅显示前 8 个，共 {channelRows.length} 个。
                </Text>
              ) : null}
              {!channels.isLoading && channelRows.length === 0 ? (
                <Text type="tertiary" size="small">
                  还没有频道，等 P3-02 的名单表单接入后在这里添加。
                </Text>
              ) : null}
            </div>
          </section>

          <section className={styles.card}>
            <div className={styles.sectionTitle}>
              <Title heading={6} style={{ margin: 0 }}>
                分区
              </Title>
              <Tag size="small">{areaRows.length} 个</Tag>
            </div>
            <div className={styles.stack}>
              {areaRows.slice(0, 8).map((area) => (
                <div key={area.id} className={styles.kv}>
                  <span className={styles.kvKey}>{area.name}</span>
                  <span className={styles.kvValue}>{area.id}</span>
                </div>
              ))}
              {areaRows.length > 8 ? (
                <Text type="tertiary" size="small">
                  仅显示前 8 个，共 {areaRows.length} 个。
                </Text>
              ) : null}
              {!areas.isLoading && areaRows.length === 0 ? (
                <Text type="tertiary" size="small">
                  还没有分区规则，可在引导流程里选择官方分区。
                </Text>
              ) : null}
            </div>
          </section>
        </div>
      </PermissionGate>
    </div>
  )
}
