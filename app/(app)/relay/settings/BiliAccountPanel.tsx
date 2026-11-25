'use client'

import { Button, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import Image from 'next/image'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import {
  relaySetupBiliRoom,
  relaySetupLoginStatus,
  relaySetupPollLogin,
  relaySetupQrCode,
  relaySetupStatus,
  type RelayQrCode,
} from '@/app/lib/relay'
import RelayError, { relayErrorMessage } from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import { FormDialog } from '@/app/ui/shell'
import { SettingsCard } from './shared'
import styles from './settings.module.scss'

const { Text, Title } = Typography

export default function BiliAccountPanel() {
  const setup = useSWR('relay:setup-status', relaySetupStatus, { revalidateOnFocus: false })
  const login = useSWR('relay:setup-login-status', relaySetupLoginStatus, {
    revalidateOnFocus: false,
  })
  const room = useSWR('relay:setup-bili-room', relaySetupBiliRoom, { revalidateOnFocus: false })
  const [qr, setQr] = useState<RelayQrCode | null>(null)
  const [pollText, setPollText] = useState('')
  const [open, setOpen] = useState(false)
  const [busy, setBusy] = useState(false)
  const { mutate: mutateLogin } = login
  const { mutate: mutateRoom } = room
  const { mutate: mutateSetup } = setup

  const startLogin = async () => {
    setBusy(true)
    setPollText('')
    try {
      const next = await relaySetupQrCode()
      setQr(next)
      setOpen(true)
    } catch (error) {
      Toast.error({ content: relayErrorMessage(error), duration: 6 })
    } finally {
      setBusy(false)
    }
  }

  useEffect(() => {
    if (!qr) return
    let cancelled = false
    let timer: number | undefined

    const poll = async () => {
      if (cancelled) return
      try {
        const result = await relaySetupPollLogin(qr.auth_code)
        if (cancelled) return
        setPollText(result.message)
        if (result.status === 'success') {
          Toast.success('B 站账号已登录，加密凭据已更新')
          setQr(null)
          setOpen(false)
          void mutateLogin()
          void mutateRoom()
          void mutateSetup()
          return
        }
        if (result.status === 'expired' || result.status === 'error') {
          setQr(null)
          setOpen(false)
          Toast.error(result.message)
          return
        }
        timer = window.setTimeout(() => void poll(), 2000)
      } catch (error) {
        if (cancelled) return
        setQr(null)
        setOpen(false)
        Toast.error({ content: relayErrorMessage(error), duration: 6 })
      }
    }

    timer = window.setTimeout(() => void poll(), 2000)
    return () => {
      cancelled = true
      if (timer !== undefined) window.clearTimeout(timer)
    }
  }, [qr, mutateLogin, mutateRoom, mutateSetup])

  return (
    <>
      <SettingsCard
        title="B 站转播账号"
        description="扫码登录写的是转播底座自己的加密 B 站凭据，和宿主 biliup 的账号登录是两套数据，互不覆盖。"
        extra={
          <Tag size="small" color={login.data?.logged_in ? 'green' : 'grey'}>
            {login.data?.logged_in ? '已登录' : '未登录'}
          </Tag>
        }
      >
        {setup.error ? (
          <RelayError
            error={setup.error}
            title="安装检查失败"
            onRetry={() => {
              void setup.mutate()
            }}
          />
        ) : null}
        {login.error ? (
          <RelayError
            error={login.error}
            title="登录状态加载失败"
            onRetry={() => {
              void login.mutate()
            }}
          />
        ) : null}
        {room.error ? (
          <RelayError
            error={room.error}
            title="房间号读取失败"
            onRetry={() => {
              void room.mutate()
            }}
          />
        ) : null}

        <dl className={relayStyles.kv}>
          <dt className={relayStyles.kvKey}>登录状态</dt>
          <dd className={relayStyles.kvValue}>{login.data?.message ?? '—'}</dd>
          <dt className={relayStyles.kvKey}>直播间号</dt>
          <dd className={relayStyles.kvValue}>
            {room.data === null ? '暂时取不到，可在向导里手填' : room.data ?? '—'}
          </dd>
          <dt className={relayStyles.kvKey}>安装检查</dt>
          <dd className={relayStyles.kvValue}>
            {setup.data
              ? setup.data.needs_setup
                ? '缺少 ' + setup.data.missing_files.join('、')
                : '配置与 Cookie 均已就绪'
              : '—'}
          </dd>
          <dt className={relayStyles.kvKey}>存储错误</dt>
          <dd className={relayStyles.kvValue}>{setup.data?.storage_error ?? '无'}</dd>
        </dl>

        <div className={styles.actions}>
          <Button theme="solid" onClick={() => void startLogin()} loading={busy}>
            扫码重新登录
          </Button>
          <Text type="tertiary" size="small">
            扫码成功后会自动刷新登录状态与房间号；已有的频道与分区配置不会变。
          </Text>
        </div>

        <Text type="tertiary" size="small">
          首次使用状态取决于配置与登录凭据是否已写入；缺文件时请先在转播向导完成首次配置。
        </Text>
      </SettingsCard>

      {open && qr ? (
        <FormDialog
          title="B 站扫码登录"
          visible={open}
          size="sm"
          footer={null}
          onCancel={() => {
            setOpen(false)
            setQr(null)
          }}
        >
          <div className={styles.qrBox}>
            <Image
              className={styles.qrImage}
              src={qr.qr_image}
              alt="B 站登录二维码"
              width={180}
              height={180}
              unoptimized
            />
            <div className={styles.field}>
              <Title heading={6} style={{ margin: 0 }}>
                用哔哩哔哩客户端扫码
              </Title>
              <Text type="tertiary" size="small">
                {pollText || '等待扫码…'}
              </Text>
              <Text type="tertiary" size="small">
                二维码过期后关闭对话框重新生成。
              </Text>
            </div>
          </div>
        </FormDialog>
      ) : null}
    </>
  )
}
