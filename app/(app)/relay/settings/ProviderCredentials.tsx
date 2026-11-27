'use client'

import { Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import {
  relayUpdateConfig,
  type RelayConfigResponse,
  type RelayUpdateConfigRequest,
} from '@/app/lib/relay'
import { relaySecretPatch } from '@/app/lib/relay-format'
import MaskedSecretInput from '@/app/ui/relay/MaskedSecretInput'
import { expectedConfigValues, type RelayConfigFormKey } from './config-expected'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'

const { Text } = Typography

export interface ProviderCredentialsProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}

export function ProviderCredentials({ data, onSaved, onReload }: ProviderCredentialsProps) {
  const [holodexKey, setHolodexKey] = useState('')
  const [holodexClear, setHolodexClear] = useState(false)
  const [riotKey, setRiotKey] = useState('')
  const [riotClear, setRiotClear] = useState(false)
  const [jwt, setJwt] = useState('')
  const [jwtClear, setJwtClear] = useState(false)
  const keys = useSettingsSave()
  const token = useSettingsSave()

  const holodexDecision = relaySecretPatch(holodexKey, data.holodex_api_key_configured)
  const riotDecision = relaySecretPatch(riotKey, data.riot_api_key_configured)

  const patch: RelayUpdateConfigRequest = {}
  const changed: RelayConfigFormKey[] = []

  if (holodexClear) {
    patch.clear_holodex_api_key = true
    changed.push('clear_holodex_api_key')
  } else if (holodexDecision.secret !== undefined) {
    patch.holodex_api_key = holodexDecision.secret
    changed.push('holodex_api_key')
  }

  if (riotClear) {
    patch.clear_riot_api_key = true
    changed.push('clear_riot_api_key')
  } else if (riotDecision.secret !== undefined) {
    patch.riot_api_key = riotDecision.secret
    changed.push('riot_api_key')
  }

  const dirty = changed.length > 0
  const jwtDirty = jwt.trim().length > 0 || jwtClear

  const saveKeys = async () => {
    if (!dirty) return
    await keys.run(
      () =>
        relayUpdateConfig({
          ...patch,
          expected: expectedConfigValues(data, changed),
          expected_secret_revision: data.secret_revision,
        }),
      onSaved,
    )
  }

  const saveToken = async () => {
    if (!jwtDirty) {
      Toast.error('先填写 Holodex 登录令牌（JWT），或勾选清除已保存的令牌')
      return
    }
    const ok = await token.run(
      () =>
        relayUpdateConfig({
          holodex_jwt: jwtClear ? '' : jwt.trim(),
          expected_secret_revision: data.secret_revision,
        }),
      onSaved,
    )
    if (ok) {
      setJwt('')
      setJwtClear(false)
    }
  }

  return (
    <>
      <SaveErrorBanner error={keys.error} onReload={onReload} />
      <SettingsCard
        title="Holodex 与 Riot 密钥"
        description="密钥保存在加密存储里；保存后只显示星号，不会再显示原文。保存时会检查是否刚被别人改过。"
      >
        <MaskedSecretInput
          id="relay-holodex-api-key"
          label="Holodex API Key"
          value={holodexKey}
          onChange={setHolodexKey}
          configured={data.holodex_api_key_configured}
          mask={data.holodex_api_key_mask}
          clearRequested={holodexClear}
          onClearChange={setHolodexClear}
          placeholder={data.holodex_api_key_configured ? '已保存，留空表示不修改' : '填写后保存'}
        />
        <MaskedSecretInput
          id="relay-riot-api-key"
          label="Riot API Key"
          value={riotKey}
          onChange={setRiotKey}
          configured={data.riot_api_key_configured}
          mask={data.riot_api_key_mask}
          clearRequested={riotClear}
          onClearChange={setRiotClear}
          placeholder={data.riot_api_key_configured ? '已保存，留空表示不修改' : '填写后保存'}
        />
        <SaveRow
          dirty={dirty}
          saving={keys.saving}
          onSave={() => {
            void saveKeys()
          }}
          onReset={() => {
            setHolodexKey('')
            setHolodexClear(false)
            setRiotKey('')
            setRiotClear(false)
            keys.clearError()
          }}
          okText="保存 API 密钥"
        />
      </SettingsCard>

      <SaveErrorBanner error={token.error} onReload={onReload} />
      <SettingsCard
        title="Holodex 登录令牌"
        description="令牌不在配置表单的三方校验键清单里，因此单独提交；填写新令牌表示替换，勾选「保存时清除已保存的内容」并保存表示移除。"
        extra={
          <Tag size="small" color={data.holodex_jwt_configured ? 'green' : 'grey'}>
            {data.holodex_jwt_configured ? '已登录' : '未配置'}
          </Tag>
        }
      >
        <MaskedSecretInput
          id="relay-holodex-jwt"
          label="Holodex JWT"
          value={jwt}
          onChange={setJwt}
          configured={data.holodex_jwt_configured}
          multiline
          rows={3}
          clearRequested={jwtClear}
          onClearChange={setJwtClear}
          placeholder={data.holodex_jwt_configured ? '已保存，填写新令牌表示替换' : '粘贴 Holodex 登录令牌'}
        />
        <Text type="tertiary" size="small">
          单独提交时不带旧值校验，若同时有其他人修改配置，以本页最后一次提交为准。
        </Text>
        <SaveRow
          dirty={jwtDirty}
          saving={token.saving}
          onSave={() => {
            void saveToken()
          }}
          onReset={() => {
            setJwt('')
            setJwtClear(false)
            token.clearError()
          }}
          okText="更新登录令牌"
        />
      </SettingsCard>
    </>
  )
}
