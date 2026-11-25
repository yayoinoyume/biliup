'use client'

/**
 * 转播总览（`/relay`）：Phase 3 的入口骨架。
 *
 * 这里只挂轻量只读的 `/v1/relay/health`、`/v1/relay/version`、`/v1/relay/status`、
 * `/v1/relay/deps/status`，用来验证「前端 → 宿主 → 转播底座」这条链路是通的；
 * 控制台与实时曲线由 P3-03 接管本文件正文。
 */

import { Tag, Typography } from '@douyinfe/semi-ui'
import { IconRadio } from '@douyinfe/semi-icons'
import useSWR from 'swr'
import PageHeader from '../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import {
  relayDepsStatus,
  relayHealth,
  relayStatus,
  relayVersion,
} from '@/app/lib/relay'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

function StatusDot({ online }: { online: boolean }) {
  return <span className={`${styles.statusDot} ${online ? styles.online : styles.offline}`} />
}

export default function RelayOverviewPage() {
  const health = useSWR('relay:health', relayHealth)
  const version = useSWR('relay:version', relayVersion)
  const status = useSWR('relay:status', relayStatus)
  const deps = useSWR('relay:deps', relayDepsStatus, { revalidateOnFocus: false })

  const snapshot = status.data
  const platforms = [
    { key: 'youtube', label: 'YouTube', data: snapshot?.youtube ?? null },
    { key: 'twitch', label: 'Twitch', data: snapshot?.twitch ?? null },
    { key: 'niconico', label: 'Niconico', data: snapshot?.niconico ?? null },
  ]

  return (
    <div className={styles.content}>
      <PageHeader
        title="转播总览"
        description="转播底座健康状况、直播状态与推流依赖"
        icon={<IconRadio />}
      />

      <PermissionGate perm="relay.view">
        <div className={styles.grid}>
          <section className={styles.card}>
            <div className={styles.sectionTitle}>
              <Title heading={6} style={{ margin: 0 }}>
                转播底座
              </Title>
              {version.data ? <Tag size="small">v{version.data}</Tag> : null}
            </div>
            {health.error ? (
              <RelayError
                error={health.error}
                title="底座不可达"
                onRetry={() => {
                  void health.mutate()
                }}
              />
            ) : (
              <dl className={styles.kv}>
                <dt className={styles.kvKey}>运行状态</dt>
                <dd className={styles.kvValue}>
                  <StatusDot online={health.data?.enabled === true} />{' '}
                  {health.isLoading ? '读取中…' : health.data?.state ?? '未知'}
                </dd>
                <dt className={styles.kvKey}>存储就绪</dt>
                <dd className={styles.kvValue}>{health.data?.storage_ready ? '是' : '否'}</dd>
                <dt className={styles.kvKey}>SQLite 驱动</dt>
                <dd className={`${styles.kvValue} ${styles.mono}`}>{health.data?.sqlite_driver ?? '—'}</dd>
                <dt className={styles.kvKey}>数据目录</dt>
                <dd className={`${styles.kvValue} ${styles.mono}`}>{health.data?.data_dir ?? '—'}</dd>
              </dl>
            )}
          </section>

          <section className={styles.card}>
            <div className={styles.sectionTitle}>
              <Title heading={6} style={{ margin: 0 }}>
                推流依赖
              </Title>
              <Tag size="small" color={deps.data?.ready ? 'green' : 'orange'}>
                {deps.data?.ready ? '可用' : '待补'}
              </Tag>
            </div>
            <dl className={styles.kv}>
              <dt className={styles.kvKey}>ffmpeg</dt>
              <dd className={styles.kvValue}>{deps.data?.ffmpeg ? '已安装' : '未检测到'}</dd>
              <dt className={styles.kvKey}>yt-dlp</dt>
              <dd className={styles.kvValue}>{deps.data?.yt_dlp ? '已安装' : '未检测到'}</dd>
              <dt className={styles.kvKey}>streamlink</dt>
              <dd className={styles.kvValue}>{deps.data?.streamlink ? '已安装' : '未检测到'}</dd>
            </dl>
            <Text type="tertiary" size="small">
              yt-dlp 与 streamlink 至少有其一即可开播。
            </Text>
          </section>
        </div>

        <section className={styles.card}>
          <div className={styles.sectionTitle}>
            <Title heading={6} style={{ margin: 0 }}>
              B 站直播间
            </Title>
            {snapshot ? (
              <Tag size="small" color={snapshot.bilibili.is_live ? 'green' : 'grey'}>
                {snapshot.bilibili.is_live ? '直播中' : '未开播'}
              </Tag>
            ) : null}
          </div>
          {status.error ? (
            <RelayError
              error={status.error}
              onRetry={() => {
                void status.mutate()
              }}
            />
          ) : (
            <dl className={styles.kv}>
              <dt className={styles.kvKey}>房间号</dt>
              <dd className={`${styles.kvValue} ${styles.mono}`}>{snapshot?.bilibili.room || '—'}</dd>
              <dt className={styles.kvKey}>标题</dt>
              <dd className={styles.kvValue}>{snapshot?.bilibili.title || '—'}</dd>
              <dt className={styles.kvKey}>分区</dt>
              <dd className={styles.kvValue}>{snapshot?.bilibili.area_name || '—'}</dd>
              <dt className={styles.kvKey}>在线人气</dt>
              <dd className={styles.kvValue}>{snapshot?.bilibili.online ?? '—'}</dd>
            </dl>
          )}
        </section>

        <div className={styles.grid}>
          {platforms.map((platform) => (
            <section key={platform.key} className={styles.card}>
              <div className={styles.sectionTitle}>
                <Title heading={6} style={{ margin: 0 }}>
                  {platform.label}
                </Title>
                <Tag size="small" color={platform.data?.is_live ? 'green' : 'grey'}>
                  {platform.data ? (platform.data.is_live ? '直播中' : '未开播') : '未配置'}
                </Tag>
              </div>
              <dl className={styles.kv}>
                <dt className={styles.kvKey}>频道</dt>
                <dd className={styles.kvValue}>{platform.data?.channel_name || '—'}</dd>
                <dt className={styles.kvKey}>监听</dt>
                <dd className={styles.kvValue}>
                  {platform.data ? (platform.data.enable_monitor ? '开启' : '关闭') : '—'}
                </dd>
              </dl>
            </section>
          ))}
        </div>

        <div className={styles.card}>
          <Text type="tertiary" size="small">
            当前为 Phase 3 底座页：频道与分区、转播设置、推流统计、实时日志分别由
            P3-02 之后的单元接管正文。
          </Text>
        </div>
      </PermissionGate>
    </div>
  )
}
