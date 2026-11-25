'use client'

/**
 * 转播页面的权限闸：按 `relay.view` / `relay.control` / `relay.config` 渲染、隐藏或提示。
 *
 * 真正的拦截在后端；这里只负责“不该看到的东西不先露出来”。
 * 权限点未加载完时先占位，避免闪一下又收起（同 `layout.tsx` 的 `pending` 处理）。
 */

import type { ReactNode } from 'react'
import { Empty, Spin } from '@douyinfe/semi-ui'
import { useMe } from '@/app/lib/use-me'
import type { Permission } from '@/app/lib/use-me'
import styles from './relay.module.scss'

export interface PermissionGateProps {
  /** 访问这一块内容需要的权限点。 */
  perm: Permission
  children: ReactNode
  /** 没权限时：`notice` 显示占位提示（默认），`hide` 什么都不渲染。 */
  mode?: 'notice' | 'hide'
  /** 权限点还在加载时的占位；默认居中转圈。 */
  loading?: ReactNode
  title?: string
  description?: string
}

const DEFAULT_DESCRIPTION = '当前账号的角色不包含这项功能，如需使用请联系超级管理员。'

export default function PermissionGate({
  perm,
  children,
  mode = 'notice',
  loading,
  title = '没有权限访问这块内容',
  description = DEFAULT_DESCRIPTION,
}: PermissionGateProps) {
  const { me, error, can } = useMe()

  if (!me && !error) {
    return (
      <div className={styles.gateLoading}>
        {loading ?? <Spin size="large" />}
      </div>
    )
  }

  if (can(perm)) return <>{children}</>
  if (mode === 'hide') return null

  return (
    <div className={styles.gateNotice}>
      <Empty title={title} description={description} />
    </div>
  )
}
