'use client'

import { Button, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayCheckNiconicoSession,
  relayNiconicoSessionStatus,
  relayUpdateConfig,
  type RelayConfigResponse,
  type RelayNiconicoSessionState,
  type RelayUpdateConfigRequest,
} from '@/app/lib/relay'
import { formatIsoDateTime, relaySecretPatch } from '@/app/lib/relay-format'
import MaskedSecretInput from '@/app/ui/relay/MaskedSecretInput'
import RelayError, { relayErrorMessage } from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import { expectedConfigValues, type RelayConfigFormKey } from './config-expected'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'
import styles from './settings.module.scss'

const { Text } = Typography

const STATE_LABEL: Record<RelayNiconicoSessionState, string> = {
  unconfigured: '未配置',
  unchecked: '未检测',
  valid: '有效',
  invalid: '已失效',
  unavailable: '暂时无法验证',
}

const STATE_COLOR: Record<RelayNiconicoSessionState, 'green' | 'red' | 'orange' | 'grey'> = {
  unconfigured: 'grey',
  unchecked: 'grey',
  valid: 'green',
  invalid: 'red',
  unavailable: 'orange',
}

export interface NiconicoSessionProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}

export default function NiconicoSession({ data, onSaved, onReload }: NiconicoSessionProps) {
  const session = useSWR('relay:niconico-session', relayNiconicoSessionStatus, {
    revalidateOnFocus: false,
  })
  const [value, setValue] = useState('')
  const [clear, setClear] = useState(false)
  const [checking, setChecking] = useState(false)
  const state = useSettingsSave()
  const status = session.data

  const decision = relaySecretPatch(value, data.niconico.user_session_configured)
  const patch: RelayUpdateConfigRequest = {}
  const changed: RelayConfigFormKey[] = []

  if (clear) {
    patch.clear_niconico_user_session = true
    changed.push('clear_niconico_user_session')
  } else if (decision.secret !== undefined) {
    patch.niconico_user_session = decision.secret
    changed.push('niconico_user_session')
  }

  const dirty = changed.length > 0

  const runCheck = async () => {
    setChecking(true)
    try {
      const next = await relayCheckNiconicoSession()
      await session.mutate(next, { revalidate: false })
      Toast.success('会话校验完成')
    } catch (error) {
      Toast.error({ content: relayErrorMessage(error), duration: 6 })
    } finally {
      setChecking(false)
    }
  }

  const save = async () => {
    if (!dirty) return
    const ok = await state.run(
      () =>
        relayUpdateConfig({
          ...patch,
          expected: expectedConfigValues(data, changed),
          expected_secret_revision: data.secret_revision,
        }),
      async () => {
        await onSaved()
        await session.mutate()
      },
    )
    if (ok) {
      setValue('')
      setClear(false)
    }
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />
      <SettingsCard
        title="Niconico 会话"
        description="user_session Cookie 的值，加密保存；状态来自最近一次校验，校验本身不会延长会话有效期。"
        extra={
          status ? (
            <Tag size="small" color={STATE_COLOR[status.state]}>
              {STATE_LABEL[status.state]}
            </Tag>
          ) : null
        }
      >
        {session.error ? (
          <RelayError
            error={session.error}
            title="Niconico 会话状态加载失败"
            onRetry={() => {
              void session.mutate()
            }}
          />
        ) : null}

        <dl className={relayStyles.kv}>
          <dt className={relayStyles.kvKey}>状态说明</dt>
          <dd className={relayStyles.kvValue}>{status?.message ?? '—'}</dd>
          <dt className={relayStyles.kvKey}>上次检测</dt>
          <dd className={relayStyles.kvValue}>{formatIsoDateTime(status?.checked_at)}</dd>
        </dl>

        <div className={styles.actions}>
          <Button onClick={() => void runCheck()} loading={checking}>
            立即校验会话
          </Button>
          <Text type="tertiary" size="small">
            会真实请求一次 Niconico；网络异常时状态是「暂时无法验证」，不代表会话失效。
          </Text>
        </div>

        <MaskedSecretInput
          id="relay-niconico-session"
          label="user_session"
          value={value}
          onChange={setValue}
          configured={data.niconico.user_session_configured}
          mask={data.niconico.user_session_mask}
          clearRequested={clear}
          onClearChange={setClear}
          placeholder={
            data.niconico.user_session_configured
              ? '已保存，留空表示不修改'
              : '只填 Cookie 的值，不要带 user_session= 前缀'
          }
          help="只接受 Cookie 值本身，不能包含空格、换行或其它 Cookie。"
        />

        <SaveRow
          dirty={dirty}
          saving={state.saving}
          onSave={() => {
            void save()
          }}
          onReset={() => {
            setValue('')
            setClear(false)
            state.clearError()
          }}
          okText="保存 Niconico 会话"
        />
      </SettingsCard>
    </>
  )
}
