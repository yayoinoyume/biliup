'use client'

import { Suspense, useCallback } from 'react'
import { Spin, TabPane, Tabs } from '@douyinfe/semi-ui'
import { IconServerStroked } from '@douyinfe/semi-icons'
import { usePathname, useRouter, useSearchParams } from 'next/navigation'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import type { RelayCropPlatform } from '@/app/lib/relay'
import AreasPanel from './AreasPanel'
import ChannelsPanel from './ChannelsPanel'
import CropPanel from './CropPanel'
import styles from '@/app/ui/relay/relay.module.scss'

type ManageTab = 'channels' | 'areas' | 'crop'

const TABS: ManageTab[] = ['channels', 'areas', 'crop']
const PLATFORMS: RelayCropPlatform[] = ['youtube', 'twitch', 'niconico']

function parseTab(value: string | null): ManageTab {
  return TABS.find((tab) => tab === value) ?? 'channels'
}

function parsePlatform(value: string | null): RelayCropPlatform {
  return PLATFORMS.find((platform) => platform === value) ?? 'youtube'
}

function ManageContent() {
  const router = useRouter()
  const pathname = usePathname()
  const searchParams = useSearchParams()
  const tab = parseTab(searchParams.get('tab'))
  const platform = parsePlatform(searchParams.get('platform'))

  const navigate = useCallback(
    (nextTab: ManageTab, nextPlatform: RelayCropPlatform) => {
      const params = new URLSearchParams()
      params.set('tab', nextTab)
      if (nextTab === 'crop') params.set('platform', nextPlatform)
      router.replace(`${pathname}?${params.toString()}`, { scroll: false })
    },
    [pathname, router],
  )

  return (
    <div className={styles.content}>
      <PageHeader
        title="频道与分区"
        description="频道名单、分区规则、画面裁剪与 HLS 时移缓存"
        icon={<IconServerStroked />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
      />

      <PermissionGate perm="relay.view">
        <Tabs type="line" activeKey={tab} onChange={(key) => navigate(parseTab(key), platform)}>
          <TabPane itemKey="channels" tab="频道管理">
            <ChannelsPanel />
          </TabPane>
          <TabPane itemKey="areas" tab="分区管理">
            <AreasPanel />
          </TabPane>
          <TabPane itemKey="crop" tab="画面裁剪与时移缓存">
            <CropPanel platform={platform} onPlatformChange={(next) => navigate('crop', next)} />
          </TabPane>
        </Tabs>
      </PermissionGate>
    </div>
  )
}

export default function RelayManagePage() {
  return (
    <Suspense
      fallback={
        <div className={styles.gateLoading}>
          <Spin size="large" />
        </div>
      }
    >
      <ManageContent />
    </Suspense>
  )
}
