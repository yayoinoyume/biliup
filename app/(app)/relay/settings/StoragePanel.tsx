'use client'

import { Banner, Button, Input, Toast, Typography } from '@douyinfe/semi-ui'
import { useRef, useState, type ChangeEvent } from 'react'
import useSWR from 'swr'
import { relayStorageBackup, relayStorageRestore, relayStorageStatus } from '@/app/lib/relay'
import { RELAY_MAX_RESTORE_BYTES, formatBytes } from '@/app/lib/relay-format'
import RelayError, { relayErrorMessage } from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import { FormDialog } from '@/app/ui/shell'
import { SettingsCard, SettingsField } from './shared'
import styles from './settings.module.scss'

const { Text } = Typography

const MIN_PASSPHRASE = 12

export interface StoragePanelProps {
  onRestored: () => Promise<unknown>
}

export default function StoragePanel({ onRestored }: StoragePanelProps) {
  const storage = useSWR('relay:storage', relayStorageStatus, { revalidateOnFocus: false })
  const inputRef = useRef<HTMLInputElement>(null)
  const [backupPhrase, setBackupPhrase] = useState('')
  const [backupBusy, setBackupBusy] = useState(false)
  const [restorePhrase, setRestorePhrase] = useState('')
  const [file, setFile] = useState<File | null>(null)
  const [restoreBusy, setRestoreBusy] = useState(false)
  const [confirmOpen, setConfirmOpen] = useState(false)
  const status = storage.data
  const ready = status?.ready === true
  const configured = status?.ready === true ? status.configured : false
  const restoreAllowed = ready && !configured

  const download = async () => {
    const phrase = backupPhrase.trim()
    if (phrase.length < MIN_PASSPHRASE) {
      Toast.error('备份口令至少 ' + MIN_PASSPHRASE + ' 位')
      return
    }
    setBackupBusy(true)
    try {
      const { blob, filename } = await relayStorageBackup(phrase)
      const url = URL.createObjectURL(blob)
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = filename
      document.body.appendChild(anchor)
      anchor.click()
      anchor.remove()
      window.setTimeout(() => URL.revokeObjectURL(url), 1000)
      setBackupPhrase('')
      Toast.success('加密备份已开始下载')
    } catch (error) {
      Toast.error({ content: relayErrorMessage(error), duration: 6 })
    } finally {
      setBackupBusy(false)
    }
  }

  const pickFile = (event: ChangeEvent<HTMLInputElement>) => {
    if (!restoreAllowed) {
      event.target.value = ''
      return
    }
    const next = event.target.files?.[0] ?? null
    if (next && next.size > RELAY_MAX_RESTORE_BYTES) {
      Toast.error('备份文件不能超过 ' + formatBytes(RELAY_MAX_RESTORE_BYTES))
      event.target.value = ''
      setFile(null)
      return
    }
    setFile(next)
  }

  const restore = async () => {
    if (!restoreAllowed || !file) return
    const phrase = restorePhrase.trim()
    if (phrase.length < MIN_PASSPHRASE) {
      Toast.error('恢复口令至少 ' + MIN_PASSPHRASE + ' 位')
      return
    }
    if (file.size > RELAY_MAX_RESTORE_BYTES) {
      Toast.error('备份文件不能超过 ' + formatBytes(RELAY_MAX_RESTORE_BYTES))
      return
    }
    setRestoreBusy(true)
    try {
      const message = await relayStorageRestore(phrase, file)
      Toast.success({ content: message ?? '备份已恢复', duration: 6 })
      setConfirmOpen(false)
      setRestorePhrase('')
      setFile(null)
      if (inputRef.current) inputRef.current.value = ''
      try {
        await storage.mutate()
        await onRestored()
      } catch (refreshError) {
        Toast.warning({
          content: '备份已恢复，但刷新配置失败：' + relayErrorMessage(refreshError),
          duration: 6,
        })
      }
    } catch (error) {
      Toast.error({ content: relayErrorMessage(error), duration: 6 })
    } finally {
      setRestoreBusy(false)
    }
  }

  return (
    <>
      <SettingsCard
        title="加密存储"
        description="转播服务的配置、登录信息与监控数据都存进加密数据库；备份会导出全部数据，恢复会覆盖当前全部数据。"
      >
        {storage.error ? (
          <RelayError
            error={storage.error}
            title="存储状态加载失败"
            onRetry={() => {
              void storage.mutate()
            }}
          />
        ) : null}

        {status?.ready === false ? (
          <Text type="danger" size="small">
            存储不可用：{status.message}
          </Text>
        ) : null}

        <dl className={relayStyles.kv}>
          <dt className={relayStyles.kvKey}>存储状态</dt>
          <dd className={relayStyles.kvValue}>{ready ? '已就绪' : '不可用'}</dd>
          <dt className={relayStyles.kvKey}>数据库</dt>
          <dd className={relayStyles.kvValue}>
            {status?.ready === true ? status.sqlite_version : '—'}
          </dd>
          <dt className={relayStyles.kvKey}>配置数据</dt>
          <dd className={relayStyles.kvValue}>
            {status?.ready === true ? (configured ? '已写入' : '尚未写入') : '—'}
          </dd>
        </dl>

        <div className={styles.formGrid}>
          <SettingsField
            label="导出备份口令"
            hint={'至少 ' + MIN_PASSPHRASE + ' 位；口令只用于加密备份，不会保存。'}
          >
            <Input
              type="password"
              value={backupPhrase}
              onChange={setBackupPhrase}
              placeholder="输入备份口令"
              autoComplete="new-password"
              disabled={!ready}
            />
          </SettingsField>
        </div>
        <div className={styles.actions}>
          <Button
            theme="solid"
            onClick={() => void download()}
            loading={backupBusy}
            disabled={!ready || backupPhrase.trim().length < MIN_PASSPHRASE}
          >
            下载加密备份
          </Button>
          <Text type="tertiary" size="small">
            导出内容包含全部数据与登录信息，请按敏感文件保管。
          </Text>
        </div>
      </SettingsCard>

      <SettingsCard
        title="恢复备份"
        description="上传不超过 32MiB 的 biliup.backup，输对口令后会覆盖全部数据；恢复无法撤销。"
      >
        {ready && configured ? (
          <Banner
            className={relayStyles.banner}
            type="warning"
            closeIcon={null}
            title="当前已有数据，不能恢复备份"
            description="当前已有数据；为了避免覆盖现有内容，只允许在全新安装或清空数据后恢复备份。如需导出当前配置，请使用上方「导出加密备份」。"
          />
        ) : null}

        <div className={styles.formGrid}>
          <SettingsField label="选择备份文件" hint={'上限 ' + formatBytes(RELAY_MAX_RESTORE_BYTES) + '。'}>
            <input
              ref={inputRef}
              className={styles.fileInput}
              type="file"
              accept=".backup,application/octet-stream"
              onChange={pickFile}
              disabled={!restoreAllowed}
            />
          </SettingsField>
          <SettingsField label="恢复口令" hint={'与导出时输入的口令一致，至少 ' + MIN_PASSPHRASE + ' 位。'}>
            <Input
              type="password"
              value={restorePhrase}
              onChange={setRestorePhrase}
              placeholder="输入备份口令"
              autoComplete="current-password"
              disabled={!restoreAllowed}
            />
          </SettingsField>
        </div>

        {file ? (
          <Text type="tertiary" size="small">
            已选择 {file.name}（{formatBytes(file.size)}）
          </Text>
        ) : null}

        <div className={styles.actions}>
          <Button
            type="danger"
            theme="solid"
            onClick={() => setConfirmOpen(true)}
            disabled={
              !restoreAllowed ||
              !file ||
              restorePhrase.trim().length < MIN_PASSPHRASE ||
              restoreBusy
            }
          >
            恢复备份
          </Button>
          <Text type="tertiary" size="small">
            恢复成功后配置与存储状态会自动刷新。
          </Text>
        </div>
      </SettingsCard>

      {confirmOpen && file ? (
        <FormDialog
          title="确认恢复备份"
          visible={confirmOpen}
          size="sm"
          okText="覆盖并恢复"
          okType="danger"
          onOk={() => restore()}
          onCancel={() => setConfirmOpen(false)}
        >
          <Text>
            将用 {file.name}（{formatBytes(file.size)}）覆盖当前全部数据，包括配置、登录信息与统计数据。恢复过程不可撤销，确定继续吗？
          </Text>
        </FormDialog>
      ) : null}
    </>
  )
}
