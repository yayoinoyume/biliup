'use client'

import { Banner, Button, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { IconLink, IconRadio } from '@douyinfe/semi-icons'
import { useState } from 'react'
import Link from 'next/link'
import useSWR, { useSWRConfig } from 'swr'
import PageHeader from '../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError, { relayErrorMessage } from '@/app/ui/relay/RelayError'
import { useMe } from '@/app/lib/use-me'
import { relaySetupStatus, relayStartStream, relayStatus } from '@/app/lib/relay'
import { useRelayEvents, type RelayConnectionState } from '@/app/lib/relay-events'
import StatusCards from './components/StatusCards'
import PriorityCard from './components/PriorityCard'
import NetworkPanel from './components/NetworkPanel'
import StreamList from './components/StreamList'
import FaceAuthModal from './components/FaceAuthModal'
import HolodexLoginModal from './components/HolodexLoginModal'
import { RELAY_EVENT_KEYS, RELAY_SETUP_KEY, RELAY_STATUS_KEY } from './components/keys'
import pageStyles from './components/relay-overview.module.scss'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text } = Typography

const CONNECTION_LABELS: Record<
  RelayConnectionState,
  { label: string; color: 'green' | 'orange' | 'red' }
> = {
  connecting: { label: '实时连接中', color: 'orange' },
  open: { label: '实时已连接', color: 'green' },
  reconnecting: { label: '实时重连中', color: 'orange' },
  offline: { label: '实时已断开', color: 'red' },
}

export default function RelayOverviewPage() {
  const { can } = useMe()
  const { mutate } = useSWRConfig()
  const [connection, setConnection] = useState<RelayConnectionState>('connecting')
  const [faceAuth, setFaceAuth] = useState<{ qrUrl: string | null; message: string } | null>(null)
  const [checkingFaceAuth, setCheckingFaceAuth] = useState(false)
  const [holodexOpen, setHolodexOpen] = useState(false)
  const status = useSWR(RELAY_STATUS_KEY, relayStatus)
  const setup = useSWR(RELAY_SETUP_KEY, relaySetupStatus, { revalidateOnFocus: false })
  const canControl = can('relay.control')
  const canConfig = can('relay.config')

  useRelayEvents({
    enabled: can('relay.view'),
    onEvent: (event) => {
      for (const key of RELAY_EVENT_KEYS[event.kind]) void mutate(key)
    },
    onConnectionChange: setConnection,
  })

  const refreshStatus = () => {
    void status.mutate()
  }

  const retryFaceAuth = async () => {
    if (checkingFaceAuth) return
    setCheckingFaceAuth(true)
    try {
      const outcome = await relayStartStream()
      if (outcome.ok) {
        Toast.success(outcome.message ?? '直播已开始')
        setFaceAuth(null)
        refreshStatus()
      } else if (outcome.faceAuth) {
        setFaceAuth({ qrUrl: outcome.faceAuth.qrUrl, message: outcome.message })
      } else {
        Toast.error(outcome.message)
        setFaceAuth(null)
      }
    } catch (error) {
      Toast.error(relayErrorMessage(error))
    } finally {
      setCheckingFaceAuth(false)
    }
  }

  return (
    <div className={styles.content}>
      <PageHeader
        title="转播总览"
        description="实时直播状态、优先频道、推流指标与 Holodex 直播预告"
        icon={<IconRadio />}
        actions={
          can('relay.view') ? (
            <div className={styles.toolbar}>
              <Tag size="small" color={CONNECTION_LABELS[connection].color}>
                {CONNECTION_LABELS[connection].label}
              </Tag>
              <Button
                size="small"
                theme="light"
                icon={<IconLink />}
                onClick={() => setHolodexOpen(true)}
              >
                Holodex 状态
              </Button>
            </div>
          ) : undefined
        }
      />

      <PermissionGate perm="relay.view">
        {setup.data?.needs_setup ? (
          canConfig ? (
            <Banner
              type="warning"
              closeIcon={null}
              title="首次配置还没有完成"
              description={
                <div className={styles.errorBody}>
                  <Text>
                    缺少：{setup.data.missing_files.join('、') || '配置文件'}。完成扫码登录与基础参数后即可开播。
                  </Text>
                  <div className={styles.errorActions}>
                    <Link href="/relay/setup" className={pageStyles.textLink}>
                      打开首次配置向导
                    </Link>
                  </div>
                </div>
              }
            />
          ) : (
            <Banner
              type="info"
              closeIcon={null}
              title="转播底座尚未初始化"
              description="请联系管理员完成首次配置；首次配置需要「修改转播配置」权限。"
            />
          )
        ) : null}
        {setup.data?.storage_error ? (
          <Banner type="danger" closeIcon={null} title="数据存储不可用" description={setup.data.storage_error} />
        ) : null}
        {setup.error ? (
          <RelayError
            error={setup.error}
            title="引导状态读取失败"
            onRetry={() => {
              void setup.mutate()
            }}
          />
        ) : null}

        <StatusCards
          data={status.data}
          error={status.error}
          loading={status.isLoading}
          refreshing={status.isValidating}
          canControl={canControl}
          onRefresh={refreshStatus}
          onFaceAuth={(qrUrl, message) => setFaceAuth({ qrUrl, message })}
        />

        <PriorityCard
          priority={status.data?.priority_channel ?? null}
          canControl={canControl}
          canConfig={canConfig}
          onRefresh={refreshStatus}
        />

        <NetworkPanel />

        <StreamList canControl={canControl} onRefresh={refreshStatus} />

        <FaceAuthModal
          visible={faceAuth !== null}
          qrUrl={faceAuth?.qrUrl ?? null}
          message={faceAuth?.message ?? ''}
          checking={checkingFaceAuth}
          onCheck={() => void retryFaceAuth()}
          onClose={() => setFaceAuth(null)}
        />

        <HolodexLoginModal visible={holodexOpen} onClose={() => setHolodexOpen(false)} />
      </PermissionGate>
    </div>
  )
}
