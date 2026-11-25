'use client'

import { useMemo, useRef, useState } from 'react'
import { Button, Form, Input, Table, Toast, Typography } from '@douyinfe/semi-ui'
import { IconDelete, IconEdit2Stroked, IconPlus, IconSearch } from '@douyinfe/semi-icons'
import type { FormApi } from '@douyinfe/semi-ui/lib/es/form'
import type { ColumnProps } from '@douyinfe/semi-ui/lib/es/table'
import useSWR from 'swr'
import { FormDialog } from '@/app/ui/shell'
import { useMe } from '@/app/lib/use-me'
import {
  relayAddChannel,
  relayDeleteChannel,
  relayManageChannels,
  relayUpdateChannelManage,
} from '@/app/lib/relay'
import type { RelayAddChannelRequest, RelayChannel } from '@/app/lib/relay'
import RelayError, { RelayEmptyState, relayErrorMessage } from '@/app/ui/relay/RelayError'
import ConfirmDialog from './ConfirmDialog'
import YoutubeResolver from './YoutubeResolver'
import { joinList, splitList } from './form-values'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

const CHANNELS_KEY = 'relay:manage-channels'

const PLATFORM_LABELS: Record<string, string> = {
  youtube: 'YouTube',
  twitch: 'Twitch',
  niconico: 'Niconico',
}

type ChannelValues = {
  name: string
  youtube: string
  twitch: string
  niconico: string
  niconico_name: string
  aliases: string
  riot_puuid: string
}

function ChannelEditor({
  channel,
  onClose,
  onSaved,
}: {
  channel: RelayChannel | null
  onClose: () => void
  onSaved: () => void
}) {
  const formApi = useRef<FormApi<ChannelValues>>(undefined)

  const submit = async () => {
    let values: ChannelValues
    try {
      values = (await formApi.current?.validate()) as ChannelValues
    } catch {
      return
    }
    const name = values.name.trim()
    if (!name) {
      formApi.current?.setError('name', '请填写频道名称')
      return
    }
    const known: Record<string, string> = {}
    const youtube = values.youtube.trim()
    const twitch = values.twitch.trim()
    const niconico = values.niconico.trim()
    if (youtube) known.youtube = youtube
    if (twitch) known.twitch = twitch
    if (niconico) known.niconico = niconico
    if (Object.keys(known).length === 0) {
      formApi.current?.setError('youtube', '至少填写一个平台的频道 ID')
      return
    }
    const platforms: Record<string, string> = {}
    for (const [key, id] of Object.entries(channel?.platforms ?? {})) {
      if (key !== 'youtube' && key !== 'twitch' && key !== 'niconico') platforms[key] = id
    }
    Object.assign(platforms, known)
    const payload: RelayAddChannelRequest = {
      name,
      aliases: splitList(values.aliases),
      platforms,
      niconico_name: values.niconico_name.trim() || null,
      riot_puuid: values.riot_puuid.trim() || null,
    }
    try {
      const message = channel
        ? await relayUpdateChannelManage(payload)
        : await relayAddChannel(payload)
      Toast.success(message ?? (channel ? '频道已更新' : '频道已新增'))
      onSaved()
    } catch (failure) {
      Toast.error(relayErrorMessage(failure))
    }
  }

  return (
    <FormDialog
      title={channel ? `编辑频道：${channel.name}` : '新增频道'}
      size="md"
      visible
      okText={channel ? '保存' : '新增'}
      onOk={submit}
      onCancel={onClose}
    >
      <Form<ChannelValues>
        getFormApi={(api) => {
          formApi.current = api
        }}
        initValues={{
          name: channel?.name ?? '',
          youtube: channel?.platforms.youtube ?? '',
          twitch: channel?.platforms.twitch ?? '',
          niconico: channel?.platforms.niconico ?? '',
          niconico_name: channel?.niconico_name ?? '',
          aliases: joinList(channel?.aliases),
          riot_puuid: channel?.riot_puuid ?? '',
        }}
        labelPosition="top"
      >
        <Form.Input
          field="name"
          label="频道名称"
          disabled={Boolean(channel)}
          rules={[{ required: true, message: '请填写频道名称' }]}
        />
        {channel ? (
          <Text type="tertiary" size="small">
            频道名是名单主键，改名请删除后重新添加。
          </Text>
        ) : null}
        <Form.Input field="youtube" label="YouTube 频道 ID" placeholder="UC 开头的频道 ID" />
        <YoutubeResolver
          onResolved={(channelId, suggestedName) => {
            formApi.current?.setValue('youtube', channelId)
            if (suggestedName && !formApi.current?.getValue('name')) {
              formApi.current?.setValue('name', suggestedName)
            }
          }}
        />
        <Form.Input field="twitch" label="Twitch 频道 ID" placeholder="twitch.tv 后面的频道名" />
        <Form.Input field="niconico" label="Niconico 频道 ID 或频道页 slug" />
        <Form.Input field="niconico_name" label="Niconico 显示名（可空）" />
        <Form.Input field="aliases" label="别名（逗号分隔）" placeholder="主播名、常见误写" />
        <Form.Input field="riot_puuid" label="Riot PUUID（可空）" />
      </Form>
      <Text type="tertiary" size="small">
        保存后名单立即写入本机 channels.json；正在进行的转播不受影响。
      </Text>
    </FormDialog>
  )
}

export default function ChannelsPanel() {
  const { can } = useMe()
  const canEdit = can('relay.config')
  const { data, error, isLoading, mutate } = useSWR(CHANNELS_KEY, relayManageChannels, {
    revalidateOnFocus: false,
  })
  const [query, setQuery] = useState('')
  const [editor, setEditor] = useState<{ mode: 'create' } | { mode: 'edit'; channel: RelayChannel } | null>(
    null,
  )
  const [deleting, setDeleting] = useState<RelayChannel | null>(null)

  const rows = useMemo(() => {
    const all = data?.channels ?? []
    const keyword = query.trim().toLowerCase()
    if (!keyword) return all
    return all.filter((channel) => {
      const haystack = [
        channel.name,
        channel.niconico_name ?? '',
        channel.riot_puuid ?? '',
        ...channel.aliases,
        ...Object.entries(channel.platforms).flatMap(([platform, id]) => [platform, id]),
      ]
        .join(' ')
        .toLowerCase()
      return haystack.includes(keyword)
    })
  }, [data, query])

  const columns: ColumnProps<RelayChannel>[] = [
    {
      title: '频道',
      dataIndex: 'name',
      width: 180,
      render: (_value, channel) => (
        <div className={styles.stack}>
          <Text strong>{channel.name}</Text>
          {channel.niconico_name ? (
            <Text type="tertiary" size="small">
              Niconico 名：{channel.niconico_name}
            </Text>
          ) : null}
        </div>
      ),
    },
    {
      title: '平台频道 ID',
      width: 280,
      render: (_value, channel) => {
        const entries = Object.entries(channel.platforms)
        if (entries.length === 0) {
          return (
            <Text type="warning" size="small">
              未绑定平台
            </Text>
          )
        }
        return (
          <div className={styles.stack}>
            {entries.map(([platform, id]) => (
              <span key={platform} className={styles.mono}>
                {PLATFORM_LABELS[platform] ?? platform}：{id}
              </span>
            ))}
          </div>
        )
      },
    },
    {
      title: '别名',
      width: 180,
      render: (_value, channel) =>
        channel.aliases.length > 0 ? (
          <Text size="small">{channel.aliases.join('，')}</Text>
        ) : (
          <Text type="tertiary" size="small">
            —
          </Text>
        ),
    },
    {
      title: 'Riot PUUID',
      width: 200,
      render: (_value, channel) =>
        channel.riot_puuid ? (
          <span className={styles.mono}>{channel.riot_puuid}</span>
        ) : (
          <Text type="tertiary" size="small">
            —
          </Text>
        ),
    },
  ]

  if (canEdit) {
    columns.push({
      title: '操作',
      width: 110,
      render: (_value, channel) => (
        <div className={styles.toolbar}>
          <Button
            size="small"
            theme="borderless"
            icon={<IconEdit2Stroked />}
            aria-label={`编辑 ${channel.name}`}
            onClick={() => setEditor({ mode: 'edit', channel })}
          />
          <Button
            size="small"
            type="danger"
            theme="borderless"
            icon={<IconDelete />}
            aria-label={`删除 ${channel.name}`}
            onClick={() => setDeleting(channel)}
          />
        </div>
      ),
    })
  }

  return (
    <div className={styles.stack}>
      {error ? (
        <RelayError
          error={error}
          title="频道名单加载失败"
          onRetry={() => {
            void mutate()
          }}
        />
      ) : null}

      <section className={styles.card}>
        <div className={styles.sectionTitle}>
          <div className={styles.stack}>
            <Title heading={6} style={{ margin: 0 }}>
              频道名单
            </Title>
            <Text type="tertiary" size="small">
              监听名单里的频道；新增 YouTube 频道时可先用下方解析器取频道 ID。
            </Text>
          </div>
          <div className={styles.toolbar}>
            <Input
              value={query}
              onChange={setQuery}
              prefix={<IconSearch />}
              placeholder="搜索名称、别名或平台 ID"
              showClear
              style={{ width: 240 }}
            />
            {canEdit ? (
              <Button
                icon={<IconPlus />}
                onClick={() => setEditor({ mode: 'create' })}
              >
                新增频道
              </Button>
            ) : null}
          </div>
        </div>

        <div className={styles.tableWrap}>
          <Table<RelayChannel>
            columns={columns}
            dataSource={rows}
            rowKey="name"
            pagination={false}
            size="middle"
            loading={isLoading}
            scroll={{ x: 950 }}
            empty={
              <RelayEmptyState
                title={query.trim() ? '没有匹配的频道' : '名单里还没有频道'}
                description={
                  query.trim()
                    ? '换个关键词，或者清空搜索框看全部频道。'
                    : canEdit
                      ? '点右上角「新增频道」把主播加进监听名单。'
                      : '当前账号只能查看，名单由有「修改转播配置」权限的账号维护。'
                }
              />
            }
          />
        </div>
      </section>

      {editor ? (
        <ChannelEditor
          key={editor.mode === 'edit' ? `edit-${editor.channel.name}` : 'create'}
          channel={editor.mode === 'edit' ? editor.channel : null}
          onClose={() => setEditor(null)}
          onSaved={() => {
            setEditor(null)
            void mutate()
          }}
        />
      ) : null}

      <ConfirmDialog
        visible={Boolean(deleting)}
        title="删除频道"
        description={
          deleting
            ? `将从监听名单删除「${deleting.name}」，它的平台绑定与别名一起移除；正在进行的转播不受影响。`
            : ''
        }
        onConfirm={async () => {
          if (!deleting) return
          try {
            const message = await relayDeleteChannel(deleting.name)
            Toast.success(message ?? '频道已删除')
            setDeleting(null)
            void mutate()
          } catch (failure) {
            Toast.error(relayErrorMessage(failure))
          }
        }}
        onCancel={() => setDeleting(null)}
      />
    </div>
  )
}
