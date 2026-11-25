'use client'

import { Banner, Button, Toast, Typography } from '@douyinfe/semi-ui'
import { useCallback, useState, type ReactNode } from 'react'
import { relayErrorMessage } from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import { enqueueRelaySave, isConflictError } from './save-queue'
import styles from './settings.module.scss'

const { Text, Title } = Typography

export interface SettingsCardProps {
  title: ReactNode
  description?: ReactNode
  extra?: ReactNode
  children: ReactNode
}

export function SettingsCard({ title, description, extra, children }: SettingsCardProps) {
  return (
    <section className={relayStyles.card}>
      <div className={relayStyles.sectionTitle}>
        <div className={styles.field}>
          <Title heading={6} style={{ margin: 0 }}>
            {title}
          </Title>
          {description ? (
            <Text type="tertiary" size="small">
              {description}
            </Text>
          ) : null}
        </div>
        {extra}
      </div>
      <div className={relayStyles.stack}>{children}</div>
    </section>
  )
}

export interface SettingsFieldProps {
  label: ReactNode
  hint?: ReactNode
  children: ReactNode
}

export function SettingsField({ label, hint, children }: SettingsFieldProps) {
  return (
    <div className={styles.field}>
      <span className={styles.fieldLabel}>{label}</span>
      {children}
      {hint ? <span className={styles.fieldHint}>{hint}</span> : null}
    </div>
  )
}

export interface SaveRowProps {
  dirty: boolean
  saving: boolean
  onSave: () => void
  onReset?: () => void
  okText?: string
  disabled?: boolean
}

export function SaveRow({ dirty, saving, onSave, onReset, okText = '保存', disabled }: SaveRowProps) {
  return (
    <div className={styles.actions}>
      <Button theme="solid" onClick={onSave} loading={saving} disabled={disabled || !dirty}>
        {okText}
      </Button>
      {onReset ? (
        <Button onClick={onReset} disabled={!dirty || saving}>
          还原
        </Button>
      ) : null}
      {dirty ? (
        <Text type="tertiary" size="small">
          有未保存的修改
        </Text>
      ) : null}
    </div>
  )
}

export interface SaveErrorBannerProps {
  error: unknown
  onReload: () => void
}

export function SaveErrorBanner({ error, onReload }: SaveErrorBannerProps) {
  if (!error) return null
  const conflict = isConflictError(error)
  return (
    <Banner
      type={conflict ? 'warning' : 'danger'}
      closeIcon={null}
      title={conflict ? '配置已被其他操作修改' : '保存失败'}
      description={
        <div className={styles.conflictBody}>
          <Text>{relayErrorMessage(error)}</Text>
          {conflict ? (
            <div className={styles.actions}>
              <Button size="small" theme="light" type="warning" onClick={onReload}>
                重新加载配置
              </Button>
            </div>
          ) : null}
        </div>
      }
    />
  )
}

export function useSettingsSave() {
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<unknown>(null)

  const run = useCallback(async (task: () => Promise<string | null>): Promise<boolean> => {
    setSaving(true)
    setError(null)
    try {
      const message = await enqueueRelaySave(task)
      if (message) Toast.success({ content: message, duration: 4 })
      return true
    } catch (nextError) {
      setError(nextError)
      return false
    } finally {
      setSaving(false)
    }
  }, [])

  return { saving, error, run, clearError: () => setError(null) }
}
