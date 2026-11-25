'use client'

/**
 * 转播设置（`/relay/settings`）：Phase 3 底座页。
 *
 * 先只读地展示设置页读模型的关键字段，证明 `/v1/relay/config` 与权限闸都接上了；
 * 完整的编辑表单（含 `expected` 三方校验与密钥 `keep_*` 语义）由 P3-04 接管。
 */

import { Tag, Typography } from '@douyinfe/semi-ui'
import { IconSetting } from '@douyinfe/semi-icons'
import useSWR from 'swr'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import { relayGetConfig } from '@/app/lib/relay'
import { formatDurationSec } from '@/app/lib/relay-format'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

export default function RelaySettingsPage() {
  const config = useSWR('relay:config', relayGetConfig, { revalidateOnFocus: false })
  const data = config.data

  return (
    <div className={styles.content}>
      <PageHeader
        title="转播设置"
        description="监听间隔、平台监听目标、代理与密钥"
        icon={<IconSetting />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
      />

      <PermissionGate perm="relay.config">
        {config.error ? (
          <RelayError
            error={config.error}
            onRetry={() => {
              void config.mutate()
            }}
          />
        ) : null}

        <section className={styles.card}>
          <div className={styles.sectionTitle}>
            <Title heading={6} style={{ margin: 0 }}>
              基础
            </Title>
            {data ? (
              <Tag size="small" type="light">
                配置版本 {data.secret_revision}
              </Tag>
            ) : null}
          </div>
          <div className={styles.statGrid}>
            <div className={styles.stat}>
              <span className={styles.statLabel}>监听间隔</span>
              <span className={styles.statValue}>{formatDurationSec(data?.interval)}</span>
            </div>
            <div className={styles.stat}>
              <span className={styles.statLabel}>直播间</span>
              <span className={styles.statValue}>{data?.bilibili.room ?? '—'}</span>
            </div>
            <div className={styles.stat}>
              <span className={styles.statLabel}>YouTube 监听</span>
              <span className={styles.statValue}>
                {data ? (data.youtube.enable_monitor ? '开' : '关') : '—'}
              </span>
            </div>
            <div className={styles.stat}>
              <span className={styles.statLabel}>Twitch 监听</span>
              <span className={styles.statValue}>
                {data ? (data.twitch.enable_monitor ? '开' : '关') : '—'}
              </span>
            </div>
          </div>
        </section>

        <section className={styles.card}>
          <div className={styles.sectionTitle}>
            <Title heading={6} style={{ margin: 0 }}>
              已配置的凭据
            </Title>
          </div>
          <dl className={styles.kv}>
            <dt className={styles.kvKey}>YouTube API Key</dt>
            <dd className={styles.kvValue}>{data?.youtube_api_key_configured ? '已配置' : '未配置'}</dd>
            <dt className={styles.kvKey}>Holodex API Key</dt>
            <dd className={styles.kvValue}>{data?.holodex_api_key_configured ? '已配置' : '未配置'}</dd>
            <dt className={styles.kvKey}>Riot API Key</dt>
            <dd className={styles.kvValue}>{data?.riot_api_key_configured ? '已配置' : '未配置'}</dd>
            <dt className={styles.kvKey}>Niconico 会话</dt>
            <dd className={styles.kvValue}>
              {data?.niconico.user_session_configured ? '已配置' : '未配置'}
            </dd>
          </dl>
          <Text type="tertiary" size="small">
            读模型只回掩码与「是否已配置」，密钥本体不会下发到浏览器。
          </Text>
        </section>
      </PermissionGate>
    </div>
  )
}
