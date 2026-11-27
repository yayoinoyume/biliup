'use client'

import { Button, Spin, TabPane, Tabs, Typography } from '@douyinfe/semi-ui'
import { IconSetting } from '@douyinfe/semi-icons'
import { useRouter, useSearchParams } from 'next/navigation'
import { Suspense, useState } from 'react'
import useSWR from 'swr'
import PageHeader from '../../components/PageHeader'
import { relayGetConfig } from '@/app/lib/relay'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import ApiKeysPanel from './ApiKeysPanel'
import AntiCollision from './AntiCollision'
import BannedKeywords from './BannedKeywords'
import BaseSettings from './BaseSettings'
import BiliAccountPanel from './BiliAccountPanel'
import NiconicoSession from './NiconicoSession'
import PlatformSettings from './PlatformSettings'
import PlayerFilter from './PlayerFilter'
import StoragePanel from './StoragePanel'
import WarningExempt from './WarningExempt'
import YoutubeCookies from './YoutubeCookies'
import styles from './settings.module.scss'

const { Text, Title } = Typography

const TAB_KEYS = ['base', 'platform', 'keys', 'account', 'rules', 'storage'] as const
type TabKey = (typeof TAB_KEYS)[number]

const isTabKey = (value: string | null): value is TabKey =>
  value !== null && TAB_KEYS.some((key) => key === value)

function ConfigUnavailable({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  const router = useRouter()
  return (
    <section className={relayStyles.card}>
      <div className={relayStyles.sectionTitle}>
        <Title heading={6} style={{ margin: 0 }}>
          还没有完成首次设置
        </Title>
      </div>
      <div className={relayStyles.stack}>
        <Text>
          配置尚不存在或读取失败。请先前往首次设置完成初始化，或在「存储与备份」里恢复备份。
        </Text>
        {error ? <RelayError error={error} title="配置读取失败" onRetry={onRetry} /> : null}
        <div className={styles.actions}>
          <Button onClick={onRetry}>重试读取配置</Button>
          <Button theme="light" onClick={() => router.push('/relay/setup')}>
            前往首次设置
          </Button>
        </div>
      </div>
    </section>
  )
}

function SettingsContent() {
  const params = useSearchParams()
  const requestedTab = params.get('tab')
  const config = useSWR('relay:config', relayGetConfig, { revalidateOnFocus: false })
  const [picked, setPicked] = useState<TabKey | null>(null)
  const data = config.data

  const reload = () => {
    void config.mutate()
  }

  const saved = () => config.mutate()

  const fallbackTab: TabKey = data ? 'base' : 'storage'
  const activeKey: TabKey = picked ?? (isTabKey(requestedTab) ? requestedTab : fallbackTab)

  if (!data && !config.error) {
    return (
      <div className={relayStyles.gateLoading}>
        <Spin size="large" />
      </div>
    )
  }

  return (
    <Tabs
      keepDOM
      type="line"
      className={styles.tabs}
      activeKey={activeKey}
      onChange={(key) => setPicked(isTabKey(key) ? key : null)}
    >
      <TabPane tab="基础设置" itemKey="base">
        <div className={styles.pane}>
          {data ? (
            <BaseSettings
              key={data.secret_revision}
              data={data}
              onSaved={saved}
              onReload={reload}
            />
          ) : (
            <ConfigUnavailable error={config.error} onRetry={reload} />
          )}
        </div>
      </TabPane>
      <TabPane tab="平台与代理" itemKey="platform">
        <div className={styles.pane}>
          {data ? (
            <PlatformSettings
              key={data.secret_revision}
              data={data}
              onSaved={saved}
              onReload={reload}
            />
          ) : (
            <ConfigUnavailable error={config.error} onRetry={reload} />
          )}
        </div>
      </TabPane>
      <TabPane tab="密钥池与登录信息" itemKey="keys">
        <div className={styles.pane}>
          {data ? (
            <ApiKeysPanel
              key={data.secret_revision}
              data={data}
              onSaved={saved}
              onReload={reload}
            />
          ) : (
            <ConfigUnavailable error={config.error} onRetry={reload} />
          )}
          <YoutubeCookies />
        </div>
      </TabPane>
      <TabPane tab="账号与会话" itemKey="account">
        <div className={styles.pane}>
          {data ? (
            <NiconicoSession
              key={data.secret_revision}
              data={data}
              onSaved={saved}
              onReload={reload}
            />
          ) : (
            <ConfigUnavailable error={config.error} onRetry={reload} />
          )}
          {data ? (
            <BiliAccountPanel
              key={data.secret_revision}
              data={data}
              onSaved={saved}
              onReload={reload}
            />
          ) : (
            <BiliAccountPanel />
          )}
        </div>
      </TabPane>
      <TabPane tab="风控与过滤" itemKey="rules">
        <div className={styles.pane}>
          {data ? (
            <AntiCollision
              key={data.secret_revision}
              data={data}
              onSaved={saved}
              onReload={reload}
            />
          ) : (
            <ConfigUnavailable error={config.error} onRetry={reload} />
          )}
          {data ? (
            <WarningExempt
              key={data.secret_revision}
              data={data}
              onSaved={saved}
              onReload={reload}
            />
          ) : null}
          <BannedKeywords />
          <PlayerFilter />
        </div>
      </TabPane>
      <TabPane tab="存储与备份" itemKey="storage">
        <div className={styles.pane}>
          <StoragePanel onRestored={saved} />
        </div>
      </TabPane>
    </Tabs>
  )
}

export default function RelaySettingsPage() {
  return (
    <div className={relayStyles.content}>
      <PageHeader
        title="转播设置"
        description="基础行为、平台目标与代理、密钥池、账号会话、风控规则与加密备份"
        icon={<IconSetting />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
      />

      <PermissionGate perm="relay.config">
        <Suspense
          fallback={
            <div className={relayStyles.gateLoading}>
              <Spin size="large" />
            </div>
          }
        >
          <SettingsContent />
        </Suspense>
      </PermissionGate>
    </div>
  )
}
