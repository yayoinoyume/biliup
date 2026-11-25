'use client'

import { useState } from 'react'
import { Button, Radio, RadioGroup, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { IconCamera, IconRefresh } from '@douyinfe/semi-icons'
import useSWR from 'swr'
import { useMe } from '@/app/lib/use-me'
import { relayGetCrop, relayRestartStream } from '@/app/lib/relay'
import type { RelayCropPlatform } from '@/app/lib/relay'
import RelayError, { relayErrorMessage } from '@/app/ui/relay/RelayError'
import ConfirmDialog from './ConfirmDialog'
import CropModal from './CropModal'
import HlsCacheEditor from './HlsCacheEditor'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

const PLATFORMS: { value: RelayCropPlatform; label: string }[] = [
  { value: 'youtube', label: 'YouTube' },
  { value: 'twitch', label: 'Twitch' },
  { value: 'niconico', label: 'Niconico' },
]

export default function CropPanel({
  platform,
  onPlatformChange,
}: {
  platform: RelayCropPlatform
  onPlatformChange: (next: RelayCropPlatform) => void
}) {
  const { can } = useMe()
  const canControl = can('relay.control')
  const crop = useSWR(['relay:crop', platform], () => relayGetCrop(platform), {
    revalidateOnFocus: false,
  })
  const cropLoading = crop.isLoading && crop.data === undefined
  const [editing, setEditing] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const [restarting, setRestarting] = useState(false)

  const restart = async () => {
    setRestarting(true)
    try {
      const message = await relayRestartStream()
      Toast.success(message ?? '已重启转播')
      setConfirming(false)
      void crop.mutate()
    } catch (failure) {
      Toast.error(relayErrorMessage(failure))
    } finally {
      setRestarting(false)
    }
  }

  return (
    <>
      <div className={styles.stack}>
        <div className={styles.toolbar}>
          <RadioGroup
            type="button"
            value={platform}
            onChange={(event) => onPlatformChange(event.target.value as RelayCropPlatform)}
            aria-label="裁剪配置的平台"
          >
            {PLATFORMS.map((item) => (
              <Radio key={item.value} value={item.value}>
                {item.label}
              </Radio>
            ))}
          </RadioGroup>
          <Text type="tertiary" size="small">
            平台切换会写进地址栏，可以直接分享到具体平台。
          </Text>
        </div>

        {crop.error ? (
          <RelayError
            error={crop.error}
            title="裁剪配置加载失败"
            onRetry={() => {
              void crop.mutate()
            }}
          />
        ) : null}

        <section className={styles.card}>
          <div className={styles.sectionTitle}>
            <div className={styles.stack}>
              <Title heading={6} style={{ margin: 0 }}>
                画面裁剪
              </Title>
              <Text type="tertiary" size="small">
                裁掉黑边、留白或预录区域；坐标按抓帧原始像素计算。
              </Text>
            </div>
            <Tag color={crop.data && !cropLoading ? 'green' : 'grey'} size="small">
              {cropLoading ? '加载中…' : crop.data ? '已启用' : '未启用'}
            </Tag>
          </div>

          <dl className={styles.kv}>
            <dt className={styles.kvKey}>裁切区域</dt>
            <dd className={styles.kvValue}>
              {cropLoading
                ? '加载中…'
                : crop.data
                  ? `${crop.data.width} × ${crop.data.height}，起点 (${crop.data.x}, ${crop.data.y})`
                  : '未配置裁剪'}
            </dd>
            <dt className={styles.kvKey}>抓帧来源</dt>
            <dd className={styles.kvValue}>
              {platform === 'niconico'
                ? '本地静帧（Niconico 不支持从 HLS 抓帧）'
                : 'yt-dlp / streamlink 取流 + ffmpeg 抓一帧'}
            </dd>
          </dl>

          <div className={styles.toolbar}>
            <Button icon={<IconCamera />} onClick={() => setEditing(true)}>
              {can('relay.config') ? '配置裁剪区域' : '查看裁剪配置'}
            </Button>
            {canControl ? (
              <Button
                theme="light"
                icon={<IconRefresh />}
                loading={restarting}
                onClick={() => setConfirming(true)}
              >
                重启转播
              </Button>
            ) : null}
          </div>
        </section>

        <HlsCacheEditor key={platform} platform={platform} />
      </div>

      {editing ? (
        <CropModal
          key={platform}
          platform={platform}
          visible
          onClose={() => {
            setEditing(false)
            void crop.mutate()
          }}
        />
      ) : null}

      {confirming ? (
        <ConfirmDialog
          visible
          title="重启转播"
          okText="重启"
          description="会停掉当前转播流并重新加载配置，观众侧短暂中断；重启不会自动接到原来的直播。"
          onConfirm={restart}
          onCancel={() => setConfirming(false)}
        />
      ) : null}
    </>
  )
}
