'use client'

import { Button, Tag, Typography } from '@douyinfe/semi-ui'
import { useRouter } from 'next/navigation'
import useSWR from 'swr'
import { relayHolodexAuthStatus } from '@/app/lib/relay'
import FormDialog from '@/app/ui/shell/FormDialog'
import RelayError from '@/app/ui/relay/RelayError'
import styles from '@/app/ui/relay/relay.module.scss'
import { RELAY_HOLODEX_AUTH_KEY } from './keys'

const { Text } = Typography

export interface HolodexLoginModalProps {
  visible: boolean
  onClose: () => void
}

export default function HolodexLoginModal({ visible, onClose }: HolodexLoginModalProps) {
  const router = useRouter()
  const { data, error, isLoading, mutate } = useSWR(
    visible ? RELAY_HOLODEX_AUTH_KEY : null,
    relayHolodexAuthStatus,
    { revalidateOnFocus: false },
  )

  const goSettings = () => {
    onClose()
    router.push('/relay/settings')
  }

  return (
    <FormDialog
      title="Holodex 登录状态"
      visible={visible}
      footerExtra={
        <Button size="small" theme="light" loading={isLoading} onClick={() => void mutate()}>
          刷新
        </Button>
      }
      okText="去配置 JWT / API Key"
      onOk={goSettings}
      onCancel={onClose}
    >
      <div className={styles.stack}>
        {error ? (
          <RelayError
            error={error}
            title="Holodex 状态读取失败"
            onRetry={() => {
              void mutate()
            }}
          />
        ) : null}
        <dl className={styles.kv}>
          <dt className={styles.kvKey}>登录状态</dt>
          <dd className={styles.kvValue}>
            {isLoading ? '读取中…' : data?.logged_in ? '已登录' : '未登录'}
            {data ? (
              <Tag size="small" color={data.logged_in ? 'green' : 'grey'} style={{ marginLeft: 8 }}>
                {data.logged_in ? 'JWT 可用' : '无 JWT'}
              </Tag>
            ) : null}
          </dd>
          <dt className={styles.kvKey}>Holodex 用户名</dt>
          <dd className={styles.kvValue}>{data?.username?.trim() || '—'}</dd>
          <dt className={styles.kvKey}>跳过 JWT 校验</dt>
          <dd className={styles.kvValue}>{data?.skip_jwt_verify ? '是' : '否'}</dd>
          <dt className={styles.kvKey}>JWT 已刷新</dt>
          <dd className={styles.kvValue}>{data?.jwt_refreshed ? '是' : '否'}</dd>
          <dt className={styles.kvKey}>JWT 已过期</dt>
          <dd className={styles.kvValue}>{data?.expired ? '是' : '否'}</dd>
          <dt className={styles.kvKey}>服务端说明</dt>
          <dd className={styles.kvValue}>{data?.message?.trim() || '—'}</dd>
        </dl>
        <Text type="tertiary" size="small">
          收藏列表与 Holodex 频道同步需要 JWT；Holodex 密钥用于公开数据。两者都在转播设置页维护。
        </Text>
      </div>
    </FormDialog>
  )
}
