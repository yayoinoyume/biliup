'use client'

import { Banner, Button, Input, Switch, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import Link from 'next/link'
import {
  relayRefreshNiconico,
  relayRefreshTwitch,
  relayRefreshYoutube,
  relayReleasePriorityWarning,
  relayRestartStream,
  relayStartStream,
  relayStopStream,
  relayToggleNiconicoMonitor,
  relayToggleTwitchMonitor,
  relayToggleYoutubeMonitor,
  relayUpdateArea,
  relayUpdateTitle,
} from '@/app/lib/relay'
import type {
  RelayBiliStatus,
  RelayNicoStatus,
  RelayStatusData,
  RelayTwStatus,
  RelayYtStatus,
} from '@/app/lib/relay'
import { formatDurationSec, formatIsoDateTime, formatUnixSec } from '@/app/lib/relay-format'
import { relayErrorMessage } from '@/app/ui/relay/RelayError'
import AreaCatalogPicker from '@/app/ui/relay/AreaCatalogPicker'
import RelayError from '@/app/ui/relay/RelayError'
import styles from '@/app/ui/relay/relay.module.scss'
import pageStyles from './relay-overview.module.scss'
import { useNow, useRelayTask } from './ui'

const { Text, Title } = Typography

type PlatformKey = 'youtube' | 'twitch' | 'niconico'
type PlatformData = RelayYtStatus | RelayTwStatus | RelayNicoStatus

const PLATFORM_ACTIONS: Record<
  PlatformKey,
  {
    label: string
    code: 'YT' | 'TW' | 'NC'
    toggle: (enabled: boolean) => Promise<string | null>
    refresh: () => Promise<string | null>
  }
> = {
  youtube: {
    label: 'YouTube',
    code: 'YT',
    toggle: relayToggleYoutubeMonitor,
    refresh: relayRefreshYoutube,
  },
  twitch: {
    label: 'Twitch',
    code: 'TW',
    toggle: relayToggleTwitchMonitor,
    refresh: relayRefreshTwitch,
  },
  niconico: {
    label: 'Niconico',
    code: 'NC',
    toggle: relayToggleNiconicoMonitor,
    refresh: relayRefreshNiconico,
  },
}

export interface StatusCardsProps {
  data: RelayStatusData | undefined
  error: unknown
  loading: boolean
  refreshing: boolean
  canControl: boolean
  onRefresh: () => void
  onFaceAuth: (qrUrl: string | null, message: string) => void
}

function BiliCard({
  bili,
  canControl,
  refreshing,
  onRefresh,
  onFaceAuth,
}: {
  bili: RelayBiliStatus
  canControl: boolean
  refreshing: boolean
  onRefresh: () => void
  onFaceAuth: (qrUrl: string | null, message: string) => void
}) {
  const { busy, run } = useRelayTask()
  const [starting, setStarting] = useState(false)
  const [editingTitle, setEditingTitle] = useState(false)
  const [titleDraft, setTitleDraft] = useState('')
  const [editingArea, setEditingArea] = useState(false)
  const [areaDraft, setAreaDraft] = useState<number | null>(null)
  const now = useNow(Boolean(bili.room_locked && bili.lock_till))
  const lockRemaining =
    bili.lock_till && now ? Math.max(0, bili.lock_till - Math.floor(now / 1000)) : null

  const startStream = async () => {
    if (starting) return
    setStarting(true)
    try {
      const outcome = await relayStartStream()
      if (outcome.ok) {
        Toast.success(outcome.message ?? '直播已开始')
        onRefresh()
      } else if (outcome.faceAuth) {
        onFaceAuth(outcome.faceAuth.qrUrl, outcome.message)
      } else {
        Toast.error(outcome.message)
      }
    } catch (error) {
      Toast.error(relayErrorMessage(error))
    } finally {
      setStarting(false)
    }
  }

  const saveTitle = async () => {
    const next = titleDraft.trim()
    if (!next) return
    const ok = await run(() => relayUpdateTitle({ title: next }), '直播标题已更新')
    if (ok) {
      setEditingTitle(false)
      onRefresh()
    }
  }

  const saveArea = async () => {
    if (areaDraft === null) return
    const ok = await run(() => relayUpdateArea({ area_id: areaDraft }), '分区已更新')
    if (ok) {
      setEditingArea(false)
      onRefresh()
    }
  }

  return (
    <section className={styles.card}>
      <div className={styles.sectionTitle}>
        <Title heading={6} style={{ margin: 0 }}>
          B 站直播间
        </Title>
        <div className={styles.toolbar}>
          <Tag size="small" color={bili.is_live ? 'green' : 'grey'}>
            {bili.is_live ? '直播中' : '未开播'}
          </Tag>
          <Tag size="small" color={bili.ffmpeg_running ? 'green' : 'grey'}>
            {bili.ffmpeg_running ? '推流中' : '未推流'}
          </Tag>
        </div>
      </div>

      {bili.room_locked ? (
        <Banner
          type="danger"
          closeIcon={null}
          className={styles.errorBanner}
          title="直播间封禁中，解封前不会尝试开播"
          description={
            <div className={styles.errorBody}>
              <Text>封禁原因由 B 站判定，转播底座无法读取具体原因。</Text>
              <Text>
                解封时间：{formatUnixSec(bili.lock_till)}
                {lockRemaining !== null ? '，剩余 ' + formatDurationSec(lockRemaining) : ''}
              </Text>
            </div>
          }
        />
      ) : null}

      <dl className={styles.kv}>
        <dt className={styles.kvKey}>房间号</dt>
        <dd className={styles.kvValue + ' ' + styles.mono}>{bili.room || '—'}</dd>
        <dt className={styles.kvKey}>标题</dt>
        <dd className={styles.kvValue}>
          {editingTitle ? (
            <div className={pageStyles.inlineEdit}>
              <Input
                value={titleDraft}
                onChange={setTitleDraft}
                maxLength={80}
                showClear
                placeholder="直播标题"
                onEnterPress={() => void saveTitle()}
              />
              <Button
                size="small"
                theme="solid"
                loading={busy}
                disabled={!titleDraft.trim()}
                onClick={() => void saveTitle()}
              >
                保存
              </Button>
              <Button size="small" onClick={() => setEditingTitle(false)}>
                取消
              </Button>
            </div>
          ) : (
            <div className={pageStyles.inlineEdit}>
              <span>{bili.title || '—'}</span>
              {canControl ? (
                <Button
                  size="small"
                  theme="borderless"
                  onClick={() => {
                    setTitleDraft(bili.title)
                    setEditingTitle(true)
                  }}
                >
                  编辑
                </Button>
              ) : null}
            </div>
          )}
        </dd>
        <dt className={styles.kvKey}>分区</dt>
        <dd className={styles.kvValue}>
          {editingArea ? (
            <div className={styles.stack}>
              <AreaCatalogPicker value={areaDraft ?? bili.area_id} onChange={setAreaDraft} />
              <div className={styles.toolbar}>
                <Button
                  size="small"
                  theme="solid"
                  loading={busy}
                  disabled={areaDraft === null}
                  onClick={() => void saveArea()}
                >
                  保存分区
                </Button>
                <Button size="small" onClick={() => setEditingArea(false)}>
                  取消
                </Button>
              </div>
            </div>
          ) : (
            <div className={pageStyles.inlineEdit}>
              <span>
                {bili.area_name || '—'}
                {bili.area_id ? '（' + bili.area_id + '）' : ''}
              </span>
              {canControl ? (
                <Button
                  size="small"
                  theme="borderless"
                  onClick={() => {
                    setAreaDraft(bili.area_id || null)
                    setEditingArea(true)
                  }}
                >
                  修改
                </Button>
              ) : null}
            </div>
          )}
        </dd>
        <dt className={styles.kvKey}>在线人气</dt>
        <dd className={styles.kvValue}>{bili.online ?? '—'}</dd>
        <dt className={styles.kvKey}>开播时间</dt>
        <dd className={styles.kvValue}>{formatUnixSec(bili.live_start_ts)}</dd>
        <dt className={styles.kvKey}>弹幕指令</dt>
        <dd className={styles.kvValue}>
          <span className={styles.statusDot + ' ' + (bili.enable_danmaku_command ? styles.online : styles.offline)} />{' '}
          {bili.enable_danmaku_command ? '已启用' : '已关闭'}
        </dd>
      </dl>

      <div className={pageStyles.actions}>
        {canControl ? (
          <>
            <Button
              theme="solid"
              type="primary"
              loading={starting}
              disabled={busy || bili.is_live || bili.room_locked}
              onClick={() => void startStream()}
            >
              开播
            </Button>
            <Button
              loading={busy}
              disabled={starting}
              onClick={() => {
                void run(() => relayStopStream(), '直播已停止').then((ok) => {
                  if (ok) onRefresh()
                })
              }}
            >
              停播
            </Button>
            <Button
              loading={busy}
              disabled={starting}
              onClick={() => {
                void run(() => relayRestartStream(), '已重启转播').then((ok) => {
                  if (ok) onRefresh()
                })
              }}
            >
              重启
            </Button>
          </>
        ) : null}
        <Button loading={refreshing} onClick={onRefresh}>
          刷新
        </Button>
      </div>
    </section>
  )
}

function PlatformCard({
  platform,
  data,
  canControl,
  onRefresh,
}: {
  platform: PlatformKey
  data: PlatformData | null
  canControl: boolean
  onRefresh: () => void
}) {
  const { busy, run } = useRelayTask()
  const meta = PLATFORM_ACTIONS[platform]
  const topic = data && 'topic' in data ? data.topic : null
  const game = data && 'game' in data ? data.game : null
  const liveId = data && 'live_id' in data ? data.live_id : null
  const scheduledStart = data && 'scheduled_start' in data ? data.scheduled_start : null

  return (
    <section className={styles.card}>
      <div className={styles.sectionTitle}>
        <Title heading={6} style={{ margin: 0 }}>
          {meta.label}
        </Title>
        <div className={styles.toolbar}>
          <Tag size="small" color={data?.is_live ? 'green' : 'grey'}>
            {data ? (data.is_live ? '直播中' : '未开播') : '未配置'}
          </Tag>
          {data ? (
            <Tag size="small" color={data.enable_monitor ? 'green' : 'orange'}>
              {data.enable_monitor ? '监听中' : '监听关闭'}
            </Tag>
          ) : null}
        </div>
      </div>

      {data ? (
        <>
          <dl className={styles.kv}>
            <dt className={styles.kvKey}>频道</dt>
            <dd className={styles.kvValue}>{data.channel_name || '—'}</dd>
            <dt className={styles.kvKey}>频道 ID</dt>
            <dd className={styles.kvValue + ' ' + styles.mono}>{data.channel_id || '—'}</dd>
            <dt className={styles.kvKey}>{platform === 'twitch' ? '游戏' : '主题'}</dt>
            <dd className={styles.kvValue}>
              {platform === 'twitch' ? game || '—' : platform === 'youtube' ? topic || '—' : liveId || '—'}
            </dd>
            <dt className={styles.kvKey}>内容标题</dt>
            <dd className={styles.kvValue}>{data.title?.trim() || '—'}</dd>
            <dt className={styles.kvKey}>分区</dt>
            <dd className={styles.kvValue}>
              {data.area_name || '—'}
              {data.area_id ? '（' + data.area_id + '）' : ''}
            </dd>
            <dt className={styles.kvKey}>画质</dt>
            <dd className={styles.kvValue}>{data.quality || '—'}</dd>
            <dt className={styles.kvKey}>裁剪 / HLS 时移</dt>
            <dd className={styles.kvValue}>
              {data.crop_enabled ? '裁剪已开启' : '裁剪未开启'} ·{' '}
              {data.ffmpeg_cache_enabled
                ? 'HLS 时移 ' + formatDurationSec(data.ffmpeg_cache_latency_secs) + ' 已开启'
                : 'HLS 时移未开启'}
            </dd>
            {scheduledStart ? (
              <>
                <dt className={styles.kvKey}>预定开始</dt>
                <dd className={styles.kvValue}>{formatIsoDateTime(scheduledStart)}</dd>
              </>
            ) : null}
          </dl>

          {data.warning_held ? (
            <Banner
              type="warning"
              closeIcon={null}
              className={styles.errorBanner}
              title="告警临时跳过中"
              description={
                <div className={styles.errorBody}>
                  <Text>该来源触发了风险告警，底座暂停了自动切源。</Text>
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
                                channel_name: data.channel_name,
                                platform: meta.code,
                              }),
                            '已解除告警跳过',
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
            {canControl ? (
              <>
                <div className={pageStyles.switchRow}>
                  <Text size="small">监控</Text>
                  <Switch
                    checked={data.enable_monitor}
                    loading={busy}
                    onChange={(checked) => {
                      void run(() => meta.toggle(checked), checked ? '已开启监听' : '已关闭监听').then(
                        (ok) => {
                          if (ok) onRefresh()
                        },
                      )
                    }}
                  />
                </div>
                <Button
                  loading={busy}
                  onClick={() => {
                    void run(() => meta.refresh(), '状态已刷新').then((ok) => {
                      if (ok) onRefresh()
                    })
                  }}
                >
                  立即刷新
                </Button>
              </>
            ) : (
              <Text size="small" type="tertiary">
                监控：{data.enable_monitor ? '已开启' : '已关闭'}（只读）
              </Text>
            )}
            <Link href="/relay/manage" className={pageStyles.textLink}>
              配置裁剪与时移
            </Link>
          </div>
        </>
      ) : (
        <div className={styles.stack}>
          <Text type="tertiary" size="small">
            这个平台还没有配置监控目标。
          </Text>
          <Link href="/relay/settings" className={pageStyles.textLink}>
            去转播设置配置
          </Link>
        </div>
      )}
    </section>
  )
}

export default function StatusCards({
  data,
  error,
  loading,
  refreshing,
  canControl,
  onRefresh,
  onFaceAuth,
}: StatusCardsProps) {
  return (
    <div className={styles.stack}>
      {error ? (
        <RelayError error={error} title="直播状态加载失败" onRetry={onRefresh} />
      ) : null}
      {loading ? (
        <section className={styles.card}>
          <Text type="tertiary">正在读取直播状态…</Text>
        </section>
      ) : null}
      {data ? (
        <>
          <BiliCard
            bili={data.bilibili}
            canControl={canControl}
            refreshing={refreshing}
            onRefresh={onRefresh}
            onFaceAuth={onFaceAuth}
          />
          <div className={styles.grid}>
            <PlatformCard
              platform="youtube"
              data={data.youtube}
              canControl={canControl}
              onRefresh={onRefresh}
            />
            <PlatformCard
              platform="twitch"
              data={data.twitch}
              canControl={canControl}
              onRefresh={onRefresh}
            />
            <PlatformCard
              platform="niconico"
              data={data.niconico}
              canControl={canControl}
              onRefresh={onRefresh}
            />
          </div>
        </>
      ) : null}
      {!data && !loading && !error ? (
        <section className={styles.card}>
          <Text type="tertiary">暂时没有状态快照。</Text>
        </section>
      ) : null}
    </div>
  )
}
