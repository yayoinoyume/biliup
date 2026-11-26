'use client'

import type { ReactNode } from 'react'
import { Banner } from '@douyinfe/semi-ui'
import { FormDialog } from '@/app/ui/shell'
import styles from '@/app/ui/relay/relay.module.scss'

export default function ConfirmDialog({
  visible,
  title,
  description,
  okText = '删除',
  onConfirm,
  onCancel,
}: {
  visible: boolean
  title: string
  description: ReactNode
  okText?: string
  onConfirm: () => Promise<void>
  onCancel: () => void
}) {
  return (
    <FormDialog
      title={title}
      size="sm"
      visible={visible}
      okText={okText}
      okType="danger"
      onOk={onConfirm}
      onCancel={onCancel}
    >
      <Banner className={styles.banner} type="warning" closeIcon={null} title="请确认" description={description} />
    </FormDialog>
  )
}
