'use client'

import { Banner, Button, Select, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import Image from 'next/image'
import { useRouter } from 'next/navigation'
import { useEffect, useState } from 'react'
import useSWR from 'swr'
import type { User } from '@/app/lib/api-streamer'
import {
  relaySetupBiliRoom,
  relaySetupLoginStatus,
  relaySetupPollLogin,
  relaySetupQrCode,
  relaySetupStatus,
  relayUpdateConfig,
  type RelayConfigResponse,
  type RelayQrCode,
} from '@/app/lib/relay'
import { useMe } from '@/app/lib/use-me'
import { useBiliUsers } from '@/app/lib/use-streamers'
import RelayError, { relayErrorMessage } from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import { FormDialog } from '@/app/ui/shell'
import { expectedConfigValues } from './config-expected'
import { SaveErrorBanner, SaveRow, SettingsCard, SettingsField, useSettingsSave } from './shared'
import styles from './settings.module.scss'

const { Text, Title } = Typography

export interface BiliAccountPanelProps {
  data?: RelayConfigResponse
  onSaved?: () => Promise<unknown>
  onReload?: () => void
}

export default function BiliAccountPanel({ data, onSaved, onReload }: BiliAccountPanelProps) {
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
  const saveState = useSettingsSave()
  const router = useRouter()
  const { can } = useMe()
  const canPickAccount = can('template.edit')
  const { biliUsers, isLoading: usersLoading, isError: usersError } = useBiliUsers()
  const accounts = biliUsers as User[]
  const [accountId, setAccountId] = useState<number | null>(data?.account_id ?? null)

  const savedAccountId = data?.account_id ?? null
  const savedAccount = accounts.find((account) => account.id === savedAccountId)
  const savedAccountLabel =
    savedAccountId === null
      ? ''
      : savedAccount?.name?.trim() || '账号 #' + savedAccountId
  const dirty = data !== undefined && accountId !== savedAccountId
  const accountsReady = canPickAccount && !usersLoading && !usersError
  const missingFromList =
    accountsReady &&
    savedAccountId !== null &&
    !accounts.some((account) => account.id === savedAccountId)
  const midMissing =
    data !== undefined && savedAccountId !== null && (data.account_mid ?? null) === null
  const savedUnavailable =
    data !== undefined && savedAccountId !== null && (midMissing || missingFromList)

  const accountOptions = accounts.map((account) => ({
    label: account.name?.trim() ? account.name : `账号 #${account.id}`,
    value: account.id,
  }))
  if (missingFromList && savedAccountId !== null) {
    accountOptions.unshift({
      label: `账号 #${savedAccountId}（已不可用）`,
      value: savedAccountId,
    })
  }

  const saveAccount = async () => {
    if (!data || !dirty) return
    await saveState.run(
      () =>
        relayUpdateConfig({
          account_id: accountId,
          expected: expectedConfigValues(data, ['account_id']),
        }),
      async () => {
        if (onSaved) await onSaved()
        void mutateLogin()
        void mutateRoom()
        void mutateSetup()
      },
    )
  }

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
      {data && onReload ? (
        <SaveErrorBanner error={saveState.error} onReload={onReload} />
      ) : null}
      <SettingsCard
        title="B 站转播账号"
        description="默认使用账号中心选中的宿主 B 站账号；扫码登录只是回退到转播底座自己的加密凭据，两套数据互不覆盖。"
        extra={
          <Tag
            size="small"
            color={savedAccountId !== null || login.data?.logged_in ? 'green' : 'grey'}
          >
            {savedAccountId !== null
              ? '已配置宿主账号'
              : login.data?.logged_in
                ? '已登录'
                : '未登录'}
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

        {data ? (
          <>
            {savedUnavailable ? (
              <Banner
                className={relayStyles.banner}
                type="warning"
                closeIcon={null}
                title="当前选中的账号已不可用"
                description="转播会回退到转播底座自己的 cookies.json。请重新选择账号，或清空后保存来固定回退。"
              />
            ) : null}
            <SettingsField
              label="转播账号"
              hint="不选 = 使用转播底座自己的 cookies.json；选了 = 使用账号中心对应账号的凭据。"
            >
              <Select
                value={accountId ?? undefined}
                optionList={accountOptions}
                placeholder="不使用账号中心（用转播底座自己的 cookies.json）"
                style={{ width: '100%' }}
                showClear
                loading={usersLoading}
                disabled={!canPickAccount}
                onChange={(value) => {
                  setAccountId(typeof value === 'number' ? value : null)
                }}
                aria-label="转播账号"
              />
            </SettingsField>
            {!canPickAccount ? (
              <Text type="tertiary" size="small">
                需要「编辑投稿模板」权限才能读取账号列表并选择转播账号。
              </Text>
            ) : null}
            {usersError ? (
              <Text type="danger" size="small">
                账号列表加载失败：{relayErrorMessage(usersError)}
              </Text>
            ) : null}
            {accountsReady && accounts.length === 0 ? (
              <Text type="tertiary" size="small">
                账号中心还没有 B 站账号；可以先去「管理账号」添加。
              </Text>
            ) : null}
            <div className={styles.actions}>
              <Button
                theme="light"
                disabled={!canPickAccount}
                onClick={() => router.push('/upload-manager')}
              >
                管理账号
              </Button>
              <Text type="tertiary" size="small">
                保存后，之后开播会使用所选账号的凭据。
              </Text>
            </div>
            <SaveRow
              dirty={dirty}
              saving={saveState.saving}
              onSave={() => {
                void saveAccount()
              }}
              onReset={() => {
                setAccountId(savedAccountId)
                saveState.clearError()
              }}
              okText="保存转播账号"
            />
          </>
        ) : null}

        <dl className={relayStyles.kv}>
          <dt className={relayStyles.kvKey}>登录状态</dt>
          <dd className={relayStyles.kvValue}>
            {savedAccountId !== null
              ? '宿主账号「' + savedAccountLabel + '」'
              : login.data?.message ?? '—'}
          </dd>
          <dt className={relayStyles.kvKey}>直播间号</dt>
          <dd className={relayStyles.kvValue}>
            {room.data === null ? '暂时取不到，可在向导里手填' : room.data ?? '—'}
          </dd>
          <dt className={relayStyles.kvKey}>安装检查</dt>
          <dd className={relayStyles.kvValue}>
            {setup.data
              ? setup.data.needs_setup
                ? '缺少 ' + setup.data.missing_files.join('、')
                : '配置与账号已就绪'
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
          首次使用状态取决于配置与账号是否已就绪；需要转播底座独立凭据时，再使用扫码登录。
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
