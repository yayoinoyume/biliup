'use client'

import { Button, Checkbox, Input, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import {
  relayResolveYoutubeChannel,
  relayToggleNiconicoMonitor,
  relayToggleTwitchMonitor,
  relayToggleYoutubeMonitor,
  relayUpdateChannel,
  type RelayConfigResponse,
  type RelayUpdateChannelRequest,
} from '@/app/lib/relay'
import { relayErrorMessage } from '@/app/ui/relay/RelayError'
import AreaCatalogPicker from '@/app/ui/relay/AreaCatalogPicker'
import ProxySettings from './ProxySettings'
import { ProviderCredentials } from './ProviderCredentials'
import { SaveErrorBanner, SaveRow, SettingsCard, SettingsField, useSettingsSave } from './shared'
import styles from './settings.module.scss'

const { Text } = Typography

export interface PlatformSettingsProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}

interface PlatformCardProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
}

function MonitorToggle({
  enabled,
  initial,
  saving,
  error,
  onChange,
  onApply,
  onReload,
}: {
  enabled: boolean
  initial: boolean
  saving: boolean
  error: unknown
  onChange: (next: boolean) => void
  onApply: () => void
  onReload: () => void
}) {
  return (
    <div className={styles.field}>
      <div className={styles.actions}>
        <Checkbox checked={enabled} onChange={(event) => onChange(event.target.checked === true)}>
          启用监听
        </Checkbox>
        <Button size="small" theme="light" onClick={onApply} loading={saving} disabled={enabled === initial}>
          应用监听开关
        </Button>
        <Text type="tertiary" size="small">
          该开关独立保存；如果和当前节点状态冲突，这个开关会被拒绝。
        </Text>
      </div>
      <SaveErrorBanner error={error} onReload={onReload} />
    </div>
  )
}

function YoutubeTargetCard({ data, onSaved }: PlatformCardProps) {
  const [enabled, setEnabled] = useState(data.youtube.enable_monitor)
  const [channelId, setChannelId] = useState(data.youtube.channel_id)
  const [channelName, setChannelName] = useState(data.youtube.channel_name)
  const [area, setArea] = useState(data.youtube.area_v2)
  const [resolving, setResolving] = useState(false)
  const toggle = useSettingsSave()
  const target = useSettingsSave()

  const patch: RelayUpdateChannelRequest = { platform: 'youtube' }
  let dirty = false
  if (channelId.trim() !== data.youtube.channel_id) {
    patch.channel_id = channelId.trim()
    dirty = true
  }
  if (channelName.trim() !== data.youtube.channel_name) {
    patch.channel_name = channelName.trim()
    dirty = true
  }
  if (area !== data.youtube.area_v2) {
    patch.area_id = area
    dirty = true
  }

  const resolve = async () => {
    const input = channelId.trim()
    if (!input) {
      Toast.error('先填写频道 ID、@handle 或频道网址')
      return
    }
    setResolving(true)
    try {
      const resolved = await relayResolveYoutubeChannel(input)
      setChannelId(resolved)
      Toast.success('已解析为频道 ID')
    } catch (error) {
      Toast.error({ content: relayErrorMessage(error), duration: 6 })
    } finally {
      setResolving(false)
    }
  }

  const saveToggle = async () => {
    await toggle.run(() => relayToggleYoutubeMonitor(enabled), onSaved)
  }

  const saveTarget = async () => {
    if (!dirty) return
    if (!channelId.trim()) {
      Toast.error('YouTube 频道 ID 不能为空')
      return
    }
    await target.run(() => relayUpdateChannel(patch), onSaved)
  }

  return (
    <SettingsCard
      title="YouTube"
      description="监听目标、转播分区与监听开关。频道 ID 支持 UC 开头，@handle 与网址可以先解析。"
      extra={
        <Tag size="small" color={data.youtube.enable_monitor ? 'green' : 'grey'}>
          {data.youtube.enable_monitor ? '监听中' : '未监听'}
        </Tag>
      }
    >
      <MonitorToggle
        enabled={enabled}
        initial={data.youtube.enable_monitor}
        saving={toggle.saving}
        error={toggle.error}
        onChange={setEnabled}
        onApply={() => {
          void saveToggle()
        }}
        onReload={() => {
          void onSaved()
        }}
      />
      <div className={styles.formGrid}>
        <SettingsField label="频道 ID">
          <Input value={channelId} onChange={setChannelId} placeholder="UC 开头的频道 ID" />
        </SettingsField>
        <SettingsField label="频道显示名" hint="只用于面板展示，不参与监听匹配。">
          <Input value={channelName} onChange={setChannelName} placeholder="频道名" />
        </SettingsField>
      </div>
      <div className={styles.actions}>
        <Button size="small" onClick={() => void resolve()} loading={resolving}>
          解析频道 ID
        </Button>
      </div>
      <SettingsField label="转播分区" hint="开播时把稿件落在哪个分区。">
        <AreaCatalogPicker value={area} onChange={setArea} />
      </SettingsField>
      <SaveErrorBanner error={target.error} onReload={() => void onSaved()} />
      <SaveRow
        dirty={dirty}
        saving={target.saving}
        onSave={() => {
          void saveTarget()
        }}
        onReset={() => {
          setChannelId(data.youtube.channel_id)
          setChannelName(data.youtube.channel_name)
          setArea(data.youtube.area_v2)
          target.clearError()
        }}
        okText="保存 YouTube 目标"
      />
    </SettingsCard>
  )
}

function TwitchTargetCard({ data, onSaved }: PlatformCardProps) {
  const [enabled, setEnabled] = useState(data.twitch.enable_monitor)
  const [channelId, setChannelId] = useState(data.twitch.channel_id)
  const [channelName, setChannelName] = useState(data.twitch.channel_name)
  const [area, setArea] = useState(data.twitch.area_v2)
  const toggle = useSettingsSave()
  const target = useSettingsSave()

  const patch: RelayUpdateChannelRequest = { platform: 'twitch' }
  let dirty = false
  if (channelId.trim() !== data.twitch.channel_id) {
    patch.channel_id = channelId.trim()
    dirty = true
  }
  if (channelName.trim() !== data.twitch.channel_name) {
    patch.channel_name = channelName.trim()
    dirty = true
  }
  if (area !== data.twitch.area_v2) {
    patch.area_id = area
    dirty = true
  }

  const saveToggle = async () => {
    await toggle.run(() => relayToggleTwitchMonitor(enabled), onSaved)
  }

  const saveTarget = async () => {
    if (!dirty) return
    if (!channelId.trim()) {
      Toast.error('Twitch 频道 ID 不能为空')
      return
    }
    await target.run(() => relayUpdateChannel(patch), onSaved)
  }

  return (
    <SettingsCard
      title="Twitch"
      description="监听目标、转播分区与监听开关。"
      extra={
        <Tag size="small" color={data.twitch.enable_monitor ? 'green' : 'grey'}>
          {data.twitch.enable_monitor ? '监听中' : '未监听'}
        </Tag>
      }
    >
      <MonitorToggle
        enabled={enabled}
        initial={data.twitch.enable_monitor}
        saving={toggle.saving}
        error={toggle.error}
        onChange={setEnabled}
        onApply={() => {
          void saveToggle()
        }}
        onReload={() => {
          void onSaved()
        }}
      />
      <div className={styles.formGrid}>
        <SettingsField label="频道 ID">
          <Input value={channelId} onChange={setChannelId} placeholder="Twitch 频道 ID" />
        </SettingsField>
        <SettingsField label="频道显示名">
          <Input value={channelName} onChange={setChannelName} placeholder="频道名" />
        </SettingsField>
      </div>
      <SettingsField label="转播分区">
        <AreaCatalogPicker value={area} onChange={setArea} />
      </SettingsField>
      <SaveErrorBanner error={target.error} onReload={() => void onSaved()} />
      <SaveRow
        dirty={dirty}
        saving={target.saving}
        onSave={() => {
          void saveTarget()
        }}
        onReset={() => {
          setChannelId(data.twitch.channel_id)
          setChannelName(data.twitch.channel_name)
          setArea(data.twitch.area_v2)
          target.clearError()
        }}
        okText="保存 Twitch 目标"
      />
    </SettingsCard>
  )
}

function NiconicoTargetCard({ data, onSaved }: PlatformCardProps) {
  const [enabled, setEnabled] = useState(data.niconico.enable_monitor)
  const [channelId, setChannelId] = useState(data.niconico.channel_id)
  const [channelName, setChannelName] = useState(data.niconico.channel_name)
  const [liveId, setLiveId] = useState(data.niconico.live_id)
  const [area, setArea] = useState(data.niconico.area_v2)
  const toggle = useSettingsSave()
  const target = useSettingsSave()

  const patch: RelayUpdateChannelRequest = { platform: 'niconico' }
  let dirty = false
  if (channelId.trim() !== data.niconico.channel_id) {
    patch.channel_id = channelId.trim()
    dirty = true
  }
  if (channelName.trim() !== data.niconico.channel_name) {
    patch.channel_name = channelName.trim()
    dirty = true
  }
  if (liveId.trim() !== data.niconico.live_id) {
    patch.live_id = liveId.trim()
    dirty = true
  }
  if (area !== data.niconico.area_v2) {
    patch.area_id = area
    dirty = true
  }

  const saveToggle = async () => {
    await toggle.run(() => relayToggleNiconicoMonitor(enabled), onSaved)
  }

  const saveTarget = async () => {
    if (!dirty) return
    if (!channelId.trim()) {
      Toast.error('Niconico 频道 ID 不能为空')
      return
    }
    await target.run(() => relayUpdateChannel(patch), onSaved)
  }

  return (
    <SettingsCard
      title="Niconico"
      description="频道 slug 与正在转播的直播 ID；留空直播 ID 时按频道当前直播处理。"
      extra={
        <Tag size="small" color={data.niconico.enable_monitor ? 'green' : 'grey'}>
          {data.niconico.enable_monitor ? '监听中' : '未监听'}
        </Tag>
      }
    >
      <MonitorToggle
        enabled={enabled}
        initial={data.niconico.enable_monitor}
        saving={toggle.saving}
        error={toggle.error}
        onChange={setEnabled}
        onApply={() => {
          void saveToggle()
        }}
        onReload={() => {
          void onSaved()
        }}
      />
      <div className={styles.formGrid}>
        <SettingsField label="频道 ID" hint="例如 vspo；粘贴频道页网址也可以。">
          <Input value={channelId} onChange={setChannelId} placeholder="频道 slug" />
        </SettingsField>
        <SettingsField label="频道显示名">
          <Input value={channelName} onChange={setChannelName} placeholder="频道名" />
        </SettingsField>
        <SettingsField label="直播 ID" hint="例如 lv1234567890；留空表示跟随频道当前直播。">
          <Input value={liveId} onChange={setLiveId} placeholder="直播 ID" />
        </SettingsField>
      </div>
      <SettingsField label="转播分区">
        <AreaCatalogPicker value={area} onChange={setArea} />
      </SettingsField>
      <SaveErrorBanner error={target.error} onReload={() => void onSaved()} />
      <SaveRow
        dirty={dirty}
        saving={target.saving}
        onSave={() => {
          void saveTarget()
        }}
        onReset={() => {
          setChannelId(data.niconico.channel_id)
          setChannelName(data.niconico.channel_name)
          setLiveId(data.niconico.live_id)
          setArea(data.niconico.area_v2)
          target.clearError()
        }}
        okText="保存 Niconico 目标"
      />
    </SettingsCard>
  )
}

export default function PlatformSettings({ data, onSaved, onReload }: PlatformSettingsProps) {
  return (
    <>
      <YoutubeTargetCard data={data} onSaved={onSaved} />
      <TwitchTargetCard data={data} onSaved={onSaved} />
      <NiconicoTargetCard data={data} onSaved={onSaved} />
      <ProxySettings data={data} onSaved={onSaved} onReload={onReload} />
      <ProviderCredentials data={data} onSaved={onSaved} onReload={onReload} />
    </>
  )
}
