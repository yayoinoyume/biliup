'use client'

import { Button, Select, Tag, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import Link from 'next/link'
import useSWR from 'swr'
import { relayHolodexAuthStatus, relayHolodexStreams, relaySwitchStream } from '@/app/lib/relay'
import type { RelayAdminStreamListItem, RelaySwitchStreamRequest } from '@/app/lib/relay'
import { formatIsoDateTime } from '@/app/lib/relay-format'
import AreaCatalogPicker from '@/app/ui/relay/AreaCatalogPicker'
import RelayError, { RelayEmptyState } from '@/app/ui/relay/RelayError'
import FormDialog from '@/app/ui/shell/FormDialog'
import styles from '@/app/ui/relay/relay.module.scss'
import pageStyles from './relay-overview.module.scss'
import { RELAY_HOLODEX_AUTH_KEY, RELAY_STREAMS_CHANNELS_KEY, RELAY_STREAMS_FAVORITES_KEY } from './keys'
import { useRelayTask } from './ui'

const { Text, Title } = Typography

type SourceKind = 'channels' | 'favorites'
type StatusKind = 'all' | 'live' | 'upcoming'

const STATUS_LABELS: Record<string, string> = {
  live: '直播中',
  upcoming: '预告',
  past: '已结束',
}

function statusColor(status: string): 'green' | 'blue' | 'grey' {
  if (status === 'live') return 'green'
  if (status === 'upcoming') return 'blue'
  return 'grey'
}

export interface StreamListProps {
  canControl: boolean
  onRefresh: () => void
}

export default function StreamList({ canControl, onRefresh }: StreamListProps) {
  const { busy, run } = useRelayTask()
  const [source, setSource] = useState<SourceKind>('channels')
  const [statusFilter, setStatusFilter] = useState<StatusKind>('all')
  const [pending, setPending] = useState<RelayAdminStreamListItem | null>(null)
  const [pendingArea, setPendingArea] = useState<number | null>(null)
  const channels = useSWR(RELAY_STREAMS_CHANNELS_KEY, () => relayHolodexStreams({}), {
    revalidateOnFocus: false,
  })
  const favorites = useSWR(
    source === 'favorites' ? RELAY_STREAMS_FAVORITES_KEY : null,
    () => relayHolodexStreams({ favorites: true }),
    { revalidateOnFocus: false },
  )
  const auth = useSWR(RELAY_HOLODEX_AUTH_KEY, relayHolodexAuthStatus, {
    revalidateOnFocus: false,
  })
  const active = source === 'favorites' ? favorites : channels
  const items = active.data?.items ?? []
  const visible = statusFilter === 'all' ? items : items.filter((item) => item.status === statusFilter)

  const switchStream = async (item: RelayAdminStreamListItem, areaId: number | undefined) => {
    const payload: RelaySwitchStreamRequest = {
      channel_id: item.channel_id,
      title: item.title,
      status: item.status,
      area_id: areaId,
      topic_id: item.topic ?? undefined,
      external_link: item.link ?? undefined,
    }
    const ok = await run(() => relaySwitchStream(payload), '已切换直播来源')
    if (ok) {
      onRefresh()
      void active.mutate()
    }
    return ok
  }

  const handleSwitch = (item: RelayAdminStreamListItem) => {
    if (item.suggested_area_id !== null && item.suggested_area_id !== undefined) {
      void switchStream(item, item.suggested_area_id)
      return
    }
    setPendingArea(null)
    setPending(item)
  }

  const confirmPending = async () => {
    if (!pending || pendingArea === null) return
    const ok = await switchStream(pending, pendingArea)
    if (ok) setPending(null)
  }

  return (
    <section className={styles.card}>
      <div className={styles.sectionTitle}>
        <Title heading={6} style={{ margin: 0 }}>
          直播与预告
        </Title>
        <div className={styles.toolbar}>
          {auth.data ? (
            <Tag size="small" color={auth.data.logged_in ? 'green' : 'orange'}>
              {auth.data.logged_in ? 'Holodex 已登录' : 'Holodex 未登录'}
            </Tag>
          ) : null}
          <Select
            size="small"
            value={source}
            onChange={(value) => setSource(value === 'favorites' ? 'favorites' : 'channels')}
            optionList={[
              { label: '监控名单', value: 'channels' },
              { label: 'Holodex 收藏', value: 'favorites' },
            ]}
            style={{ width: 140 }}
          />
          <Select
            size="small"
            value={statusFilter}
            onChange={(value) => {
              if (value === 'live' || value === 'upcoming') setStatusFilter(value)
              else setStatusFilter('all')
            }}
            optionList={[
              { label: '全部状态', value: 'all' },
              { label: '直播中', value: 'live' },
              { label: '预告', value: 'upcoming' },
            ]}
            style={{ width: 120 }}
          />
          <Button
            size="small"
            loading={active.isValidating}
            onClick={() => {
              void active.mutate()
            }}
          >
            刷新
          </Button>
        </div>
      </div>

      {auth.data && !auth.data.logged_in ? (
        <Text type="tertiary" size="small">
          收藏列表需要 Holodex JWT，<Link href="/relay/settings">去转播设置配置</Link>。
        </Text>
      ) : null}
      {auth.error ? (
        <Text type="tertiary" size="small">
          Holodex 登录状态读取失败，不影响监控名单。
        </Text>
      ) : null}

      {active.error ? (
        <RelayError
          error={active.error}
          title="直播与预告加载失败"
          onRetry={() => {
            void active.mutate()
          }}
        />
      ) : null}

      {active.isLoading ? (
        <Text type="tertiary" size="small">
          正在读取 {source === 'favorites' ? 'Holodex 收藏' : '监控名单'}…
        </Text>
      ) : null}

      {!active.isLoading && !active.error && visible.length === 0 ? (
        <RelayEmptyState
          title="没有匹配的直播"
          description={
            source === 'favorites'
              ? '收藏列表为空，或 Holodex JWT 尚未配置。'
              : '监控名单里暂时没有直播或预告，稍后刷新看看。'
          }
        />
      ) : null}

      <div className={pageStyles.scrollList}>
        {visible.map((item) => (
          <div key={item.id} className={pageStyles.listItem}>
            <div className={pageStyles.itemMain}>
              <div className={pageStyles.itemTitleRow}>
                <Text strong ellipsis={{ showTooltip: true }}>
                  {item.title?.trim() || '未命名直播'}
                </Text>
                <Tag size="small" color={statusColor(item.status)}>
                  {STATUS_LABELS[item.status] ?? item.status}
                </Tag>
                {item.is_placeholder ? (
                  <Tag size="small" color="grey">
                    预告占位
                  </Tag>
                ) : null}
                {item.live_viewers !== null && item.live_viewers !== undefined ? (
                  <Tag size="small" color="cyan">
                    {item.live_viewers} 人
                  </Tag>
                ) : null}
              </div>
              <div className={pageStyles.itemMeta}>
                {item.channel_name}
                {item.channel_id ? ' · ' + item.channel_id : ''}
              </div>
              <div className={pageStyles.itemMeta}>
                开始：{formatIsoDateTime(item.start_actual ?? item.start_scheduled ?? item.available_at)}
                {' · '}
                建议分区：{item.suggested_area_name?.trim() || (item.suggested_area_id ? String(item.suggested_area_id) : '无')}
              </div>
              {item.link ? (
                <Link href={item.link} target="_blank" rel="noreferrer" className={pageStyles.textLink}>
                  打开原直播
                </Link>
              ) : null}
            </div>
            {canControl && item.status !== 'past' && !item.is_placeholder ? (
              <Button size="small" theme="light" disabled={busy} onClick={() => handleSwitch(item)}>
                切台
              </Button>
            ) : null}
          </div>
        ))}
      </div>

      <FormDialog
        title="选择切台分区"
        visible={pending !== null}
        okText="切台"
        okDisabled={pendingArea === null}
        confirmLoading={busy}
        onOk={confirmPending}
        onCancel={() => setPending(null)}
      >
        <div className={styles.stack}>
          <Text type="tertiary" size="small">
            {pending ? pending.title + ' · ' + pending.channel_name : ''}
          </Text>
          <AreaCatalogPicker
            value={pendingArea ?? pending?.suggested_area_id ?? null}
            onChange={setPendingArea}
          />
          <Text type="tertiary" size="small">
            这一场没有建议分区，选定后才会切换监听目标。
          </Text>
        </div>
      </FormDialog>
    </section>
  )
}
