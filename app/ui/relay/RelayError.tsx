'use client'

/**
 * 转播页面的统一错误横幅与空态：所有 P3-0x 页面共用同一套文案与重试入口。
 */

import { Banner, Button, Empty, Typography } from '@douyinfe/semi-ui'
import type { ReactNode } from 'react'
import { isRelayApiError } from '@/app/lib/relay'
import styles from './relay.module.scss'

const { Text } = Typography

/** 把任意抛出物压成一句可直接展示的中文。 */
export function relayErrorMessage(error: unknown): string {
  if (isRelayApiError(error)) return error.message
  if (error instanceof Error && error.message.trim()) return error.message
  return '请求失败，请稍后重试'
}

export interface RelayErrorProps {
  error: unknown
  /** 有就显示重试按钮。 */
  onRetry?: () => void
  retryLabel?: string
  /** 横幅标题，默认「加载失败」。 */
  title?: string
  /** 额外的排查线索，例如 ETag 冲突时的“请重新加载后再保存”。 */
  hint?: ReactNode
}

/** 统一的错误横幅：一句标题 + 服务端原文 + 可选重试。 */
export default function RelayError({
  error,
  onRetry,
  retryLabel = '重试',
  title = '加载失败',
  hint,
}: RelayErrorProps) {
  const message = relayErrorMessage(error)
  return (
    <Banner
      type="danger"
      className={styles.errorBanner}
      closeIcon={null}
      title={title}
      description={
        <div className={styles.errorBody}>
          <Text>{message}</Text>
          {hint ? <Text type="tertiary" size="small">{hint}</Text> : null}
          {onRetry ? (
            <div className={styles.errorActions}>
              <Button size="small" type="danger" theme="light" onClick={onRetry}>
                {retryLabel}
              </Button>
            </div>
          ) : null}
        </div>
      }
    />
  )
}

export interface RelayEmptyStateProps {
  title: string
  description?: ReactNode
  action?: ReactNode
}

/** 空态：没有数据时也要说清下一步做什么。 */
export function RelayEmptyState({ title, description, action }: RelayEmptyStateProps) {
  return (
    <div className={styles.emptyState}>
      <Empty title={title} description={description} />
      {action ? <div className={styles.emptyAction}>{action}</div> : null}
    </div>
  )
}
