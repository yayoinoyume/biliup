'use client'

import { Banner, Button, Input, Select, Switch, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayChannels,
  relayGetConfig,
  relayRefreshPriorityChannel,
  relayReleasePriorityWarning,
  relayUpdatePriorityChannel,
} from '@/app/lib/relay'
import type { RelayPriorityChannelStatus, RelayUpdatePriorityChannelRequest } from '@/app/lib/relay'
import AreaCatalogPicker from '@/app/ui/relay/AreaCatalogPicker'
import RelayError from '@/app/ui/relay/RelayError'
import FormDialog from '@/app/ui/shell/FormDialog'
import styles from '@/app/ui/relay/relay.module.scss'
import pageStyles from './relay-overview.module.scss'
import { RELAY_CHANNELS_KEY, RELAY_CONFIG_KEY } from './keys'
import { Field, useRelayTask } from './ui'

const { Text, Title } = Typography

export interface PriorityCardProps {
  priority: RelayPriorityChannelStatus | null
  canControl: boolean
  canConfig: boolean
  onRefresh: () => void
}

export default function PriorityCard({
  priority,
  canControl,
  canConfig,
  onRefresh,
}: PriorityCardProps) {
  const { busy, run } = useRelayTask()
  const config = useSWR(RELAY_CONFIG_KEY, relayGetConfig, { revalidateOnFocus: false })
  const channels = useSWR(RELAY_CHANNELS_KEY, relayChannels, { revalidateOnFocus: false })
  const [editing, setEditing] = useState(false)
  const [enabled, setEnabled] = useState(false)
  const [channelName, setChannelName] = useState('')
  const [defaultArea, setDefaultArea] = useState<number | null>(null)
  const [autoRestart, setAutoRestart] = useState(false)
  const [saving, setSaving] = useState(false)

  const baseline = config.data?.priority_channel
  const currentName = priority?.channel_name?.trim() || baseline?.channel_name?.trim() || ''
  const currentArea = priority?.default_area ?? baseline?.default_area ?? 0
  const currentAreaName = priority?.default_area_name?.trim() || ''
  const optionNames = (channels.data?.channels ?? [])
    .map((channel) => channel.name.trim())
    .filter((name) => name.length > 0)
  const options =
    channelName.trim() && !optionNames.includes(channelName.trim())
      ? [channelName.trim(), ...optionNames]
      : optionNames

  const openEditor = () => {
    setEnabled(priority?.enabled ?? baseline?.enabled ?? false)
    setChannelName(currentName)
    setDefaultArea(currentArea > 0 ? currentArea : null)
    setAutoRestart(priority?.auto_restart ?? baseline?.auto_restart ?? false)
    setEditing(true)
  }

  const save = async () => {
    if (saving) return
    const name = channelName.trim()
    if (enabled && !name) {
      Toast.error('启用优先频道时必须填写频道名')
      return
    }
    if (defaultArea === null) {
      Toast.error('请选择默认分区')
      return
    }
    const expected = {
      enabled: baseline?.enabled ?? priority?.enabled ?? false,
      auto_restart: baseline?.auto_restart ?? priority?.auto_restart ?? false,
      channel_name: baseline?.channel_name ?? priority?.channel_name ?? '',
      default_area: baseline?.default_area ?? priority?.default_area ?? 0,
    }
    const payload: RelayUpdatePriorityChannelRequest = {
      expected,
      enabled,
      auto_restart: autoRestart,
      default_area: defaultArea,
    }
    if (name) payload.channel_name = name
    setSaving(true)
    try {
      const message = await relayUpdatePriorityChannel(payload)
      Toast.success(message ?? '优先频道配置已更新')
      setEditing(false)
      onRefresh()
      void config.mutate()
    } catch (saveError) {
      Toast.error(saveError instanceof Error ? saveError.message : '优先频道保存失败')
      throw saveError
    } finally {
      setSaving(false)
    }
  }

  return (
    <section className={styles.card}>
      <div className={styles.sectionTitle}>
        <Title heading={6} style={{ margin: 0 }}>
          优先频道
        </Title>
        <div className={styles.toolbar}>
          <Tag size="small" color={priority?.enabled ? 'green' : priority ? 'grey' : 'orange'}>
            {priority ? (priority.enabled ? '已启用' : '已关闭') : '未配置'}
          </Tag>
          {priority ? (
            <Tag size="small" color={priority.is_live ? 'green' : 'grey'}>
              {priority.is_live ? '直播中' : '未开播'}
            </Tag>
          ) : null}
          {priority?.warning_held ? (
            <Tag size="small" color="orange">
              告警跳过中
            </Tag>
          ) : null}
        </div>
      </div>

      {config.error ? <RelayError error={config.error} title="优先频道配置加载失败" /> : null}

      <dl className={styles.kv}>
        <dt className={styles.kvKey}>频道</dt>
        <dd className={styles.kvValue}>{priority?.channel_name?.trim() || currentName || '—'}</dd>
        <dt className={styles.kvKey}>平台</dt>
        <dd className={styles.kvValue}>{priority?.platform?.trim() || '—'}</dd>
        <dt className={styles.kvKey}>正在转播</dt>
        <dd className={styles.kvValue}>{priority?.title?.trim() || '—'}</dd>
        <dt className={styles.kvKey}>默认分区</dt>
        <dd className={styles.kvValue}>
          {currentAreaName || '—'}
          {currentArea > 0 ? '（' + currentArea + '）' : ''}
        </dd>
        <dt className={styles.kvKey}>自动重启</dt>
        <dd className={styles.kvValue}>
          {(priority?.auto_restart ?? baseline?.auto_restart) ? '已开启' : '已关闭'}
        </dd>
      </dl>

      {priority?.warning_held ? (
        <Banner
          type="warning"
          closeIcon={null}
          className={styles.errorBanner}
          title="优先频道告警临时跳过中"
          description={
            <div className={styles.errorBody}>
              <Text>优先频道触发了风险告警，底座暂停了自动切源。</Text>
              {canControl ? (
                <div className={styles.errorActions}>
                  <Button
                    size="small"
                    theme="light"
                    loading={busy}
                    onClick={() => {
                      void run(
                        () =>
                          relayReleasePriorityWarning({
                            channel_name: priority.channel_name,
                            platform: 'priority',
                          }),
                        '已解除优先频道告警跳过',
                      ).then((ok) => {
                        if (ok) onRefresh()
                      })
                    }}
                  >
                    解除临时跳过
                  </Button>
                </div>
              ) : null}
            </div>
          }
        />
      ) : null}

      <div className={pageStyles.actions}>
        {canConfig ? (
          <Button theme="solid" onClick={openEditor}>
            {priority ? '编辑配置' : '配置优先频道'}
          </Button>
        ) : null}
        {canControl ? (
          <Button
            loading={busy}
            onClick={() => {
              void run(() => relayRefreshPriorityChannel(), '优先频道状态已刷新').then((ok) => {
                if (ok) onRefresh()
              })
            }}
          >
            立即刷新
          </Button>
        ) : null}
      </div>

      <FormDialog
        title="优先频道配置"
        visible={editing}
        okText="保存"
        okDisabled={!baseline}
        confirmLoading={saving}
        onOk={save}
        onCancel={() => setEditing(false)}
      >
        <div className={styles.stack}>
          {!baseline ? (
            <Text type="tertiary" size="small">
              正在读取当前配置，读取完成前不能保存。
            </Text>
          ) : null}
          <Field label="启用优先频道">
            <Switch checked={enabled} onChange={setEnabled} />
          </Field>
          <Field label="从频道名单选择" hint="保存后会自动按名字解析各平台频道 ID。">
            <Select
              value={channelName || undefined}
              onChange={(value) => setChannelName(value ? String(value) : '')}
              optionList={options.map((name) => ({ label: name, value: name }))}
              filter
              placeholder="选择频道"
              style={{ width: '100%' }}
            />
          </Field>
          <Field label="频道名" hint="名单里没有该频道时可直接填写。">
            <Input value={channelName} onChange={setChannelName} placeholder="频道名" />
          </Field>
          <Field label="默认分区">
            <AreaCatalogPicker value={defaultArea} onChange={setDefaultArea} />
          </Field>
          <Field label="自动重启">
            <Switch checked={autoRestart} onChange={setAutoRestart} />
          </Field>
        </div>
      </FormDialog>
    </section>
  )
}
