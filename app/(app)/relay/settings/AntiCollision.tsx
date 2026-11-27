'use client'

import { Button, Checkbox, Select, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayAreas,
  relayManageChannels,
  relayUpdateConfig,
  type RelayChannelsDocument,
  type RelayConfigResponse,
  type RelayUpdateConfigRequest,
} from '@/app/lib/relay'
import AreaCatalogPicker from '@/app/ui/relay/AreaCatalogPicker'
import RelayError from '@/app/ui/relay/RelayError'
import { expectedConfigValues, type RelayConfigFormKey } from './config-expected'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'
import styles from './settings.module.scss'

const { Text } = Typography

export interface AntiCollisionProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}

const sortEntries = (value: Record<string, number>): [string, number][] =>
  Object.entries(value).sort((left, right) => left[0].localeCompare(right[0], 'zh-Hans-CN'))

function AntiCollisionForm({
  data,
  channels,
  onSaved,
  onReload,
}: {
  data: RelayConfigResponse
  channels: RelayChannelsDocument | undefined
  onSaved: () => Promise<unknown>
  onReload: () => void
}) {
  const [loaded, setLoaded] = useState(data)
  const [enabled, setEnabled] = useState(data.enable_anti_collision)
  const [list, setList] = useState<Record<string, number>>(data.anti_collision_list)
  const [pick, setPick] = useState<string | null>(null)
  const [area, setArea] = useState<number | null>(null)
  const state = useSettingsSave()
  const areas = useSWR('relay:areas', relayAreas, { revalidateOnFocus: false })

  if (loaded !== data) {
    setLoaded(data)
    setEnabled(data.enable_anti_collision)
    setList(data.anti_collision_list)
  }

  const channelNames = (channels?.channels ?? []).map((channel) => channel.name)
  const areaName = (id: number): string =>
    areas.data?.areas?.find((entry) => entry.id === id)?.name ?? String(id)

  const listDirty =
    JSON.stringify(sortEntries(list)) !== JSON.stringify(sortEntries(data.anti_collision_list))
  const dirty = enabled !== data.enable_anti_collision || listDirty

  const patch: RelayUpdateConfigRequest = {}
  const changed: RelayConfigFormKey[] = []

  if (enabled !== data.enable_anti_collision) {
    patch.enable_anti_collision = enabled
    changed.push('enable_anti_collision')
  }
  if (listDirty) {
    patch.anti_collision_list = list
    changed.push('anti_collision_list')
  }

  const add = () => {
    const name = pick?.trim()
    if (!name) {
      Toast.error('先选择一个频道')
      return
    }
    if (Object.prototype.hasOwnProperty.call(list, name)) {
      Toast.error('该频道已经在防撞车名单里')
      return
    }
    if (area === null) {
      Toast.error('先选择这个频道的默认分区')
      return
    }
    setList({ ...list, [name]: area })
    setPick(null)
    setArea(null)
  }

  const remove = (name: string) => {
    const next = { ...list }
    delete next[name]
    setList(next)
  }

  const save = async () => {
    if (!dirty) return
    await state.run(
      () =>
        relayUpdateConfig({
          ...patch,
          expected: expectedConfigValues(data, changed),
        }),
      onSaved,
    )
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />
      <div className={styles.checkRow}>
        <Checkbox
          checked={enabled}
          onChange={(event) => setEnabled(event.target.checked === true)}
        >
          启用防撞车检测
        </Checkbox>
        <Text type="tertiary" size="small">
          名单里的频道开播时优先占用默认分区，避免和正在转播的频道撞到同一分区。
        </Text>
      </div>

      <ul className={styles.list}>
        {sortEntries(list).map(([name, areaId]) => (
          <li key={name} className={styles.listRow}>
            <div className={styles.listMain}>
              <Text strong>{name}</Text>
              <Tag size="small" type="light">
                {areaName(areaId)}（{areaId}）
              </Tag>
            </div>
            <Button size="small" type="danger" theme="borderless" onClick={() => remove(name)}>
              移除
            </Button>
          </li>
        ))}
        {Object.keys(list).length === 0 ? (
          <li className={styles.listRow}>
            <Text type="tertiary" size="small">
              名单还是空的，选一个频道和分区加进来。
            </Text>
          </li>
        ) : null}
      </ul>

      <div className={styles.formGrid}>
        <div className={styles.field}>
          <span className={styles.fieldLabel}>频道</span>
          <Select
            value={pick ?? undefined}
            onChange={(next) => setPick(next ? String(next) : null)}
            optionList={channelNames.map((name) => ({ label: name, value: name }))}
            filter
            showClear
            style={{ width: '100%' }}
            placeholder={channelNames.length ? '搜索并选择频道' : '先在频道与分区页面添加频道'}
            disabled={channelNames.length === 0}
          />
        </div>
        <div className={styles.field}>
          <span className={styles.fieldLabel}>默认分区</span>
          <AreaCatalogPicker value={area} onChange={setArea} />
        </div>
      </div>

      <div className={styles.actions}>
        <Button onClick={add}>加入名单</Button>
      </div>

      <SaveRow
          dirty={dirty}
          saving={state.saving}
          onSave={() => {
            void save()
          }}
          onReset={() => {
            setEnabled(data.enable_anti_collision)
            setList(data.anti_collision_list)
            state.clearError()
          }}
          okText="保存防撞车设置"
      />
    </>
  )
}

export default function AntiCollision({ data, onSaved, onReload }: AntiCollisionProps) {
  const channels = useSWR('relay:manage-channels', relayManageChannels, {
    revalidateOnFocus: false,
  })

  return (
    <SettingsCard
      title="防撞车名单"
      description="频道名到默认分区的映射；保存时只提交改过的设置；如果别人刚改过，会提示你先刷新。"
    >
      {channels.error ? (
        <RelayError
          error={channels.error}
          title="频道名单加载失败"
          onRetry={() => {
            void channels.mutate()
          }}
        />
      ) : null}
      <AntiCollisionForm
        data={data}
        channels={channels.data}
        onSaved={onSaved}
        onReload={onReload}
      />
    </SettingsCard>
  )
}
