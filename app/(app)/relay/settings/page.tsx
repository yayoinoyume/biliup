'use client'

import { Spin, TabPane, Tabs } from '@douyinfe/semi-ui'
import { IconSetting } from '@douyinfe/semi-icons'
import { useState } from 'react'
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

export default function RelaySettingsPage() {
  const config = useSWR('relay:config', relayGetConfig, { revalidateOnFocus: false })
  const [tab, setTab] = useState('base')
  const data = config.data

  const reload = () => {
    void config.mutate()
  }

  const saved = () => config.mutate()

  return (
    <div className={relayStyles.content}>
      <PageHeader
        title="转播设置"
        description="基础行为、平台目标与代理、密钥池、账号会话、风控规则与加密备份"
        icon={<IconSetting />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
      />

      <PermissionGate perm="relay.config">
        {config.error ? <RelayError error={config.error} onRetry={reload} /> : null}
        {!data && !config.error ? (
          <div className={relayStyles.gateLoading}>
            <Spin size="large" />
          </div>
        ) : null}

        {data ? (
          <Tabs
            keepDOM
            type="line"
            className={styles.tabs}
            activeKey={tab}
            onChange={setTab}
          >
            <TabPane tab="基础设置" itemKey="base">
              <div className={styles.pane}>
                <BaseSettings
                  key={data.secret_revision}
                  data={data}
                  onSaved={saved}
                  onReload={reload}
                />
              </div>
            </TabPane>
            <TabPane tab="平台与代理" itemKey="platform">
              <div className={styles.pane}>
                <PlatformSettings
                  key={data.secret_revision}
                  data={data}
                  onSaved={saved}
                  onReload={reload}
                />
              </div>
            </TabPane>
            <TabPane tab="密钥池与 Cookie" itemKey="keys">
              <div className={styles.pane}>
                <ApiKeysPanel
                  key={data.secret_revision}
                  data={data}
                  onSaved={saved}
                  onReload={reload}
                />
                <YoutubeCookies />
              </div>
            </TabPane>
            <TabPane tab="账号与会话" itemKey="account">
              <div className={styles.pane}>
                <NiconicoSession
                  key={data.secret_revision}
                  data={data}
                  onSaved={saved}
                  onReload={reload}
                />
                <BiliAccountPanel />
              </div>
            </TabPane>
            <TabPane tab="风控与过滤" itemKey="rules">
              <div className={styles.pane}>
                <AntiCollision
                  key={data.secret_revision}
                  data={data}
                  onSaved={saved}
                  onReload={reload}
                />
                <WarningExempt
                  key={data.secret_revision}
                  data={data}
                  onSaved={saved}
                  onReload={reload}
                />
                <BannedKeywords key={data.secret_revision} />
                <PlayerFilter key={data.secret_revision} />
              </div>
            </TabPane>
            <TabPane tab="存储与备份" itemKey="storage">
              <div className={styles.pane}>
                <StoragePanel onRestored={saved} />
              </div>
            </TabPane>
          </Tabs>
        ) : null}
      </PermissionGate>
    </div>
  )
}
