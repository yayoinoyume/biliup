'use client'

import { Checkbox, Input, InputNumber, Toast } from '@douyinfe/semi-ui'
import { useState } from 'react'
import {
  relayUpdateConfig,
  type RelayConfigResponse,
  type RelayUpdateConfigRequest,
} from '@/app/lib/relay'
import {
  buildProxyPatch,
  expectedConfigValues,
  type ProxyPatchResult,
  type RelayConfigFormKey,
} from './config-expected'
import { SaveErrorBanner, SaveRow, SettingsCard, SettingsField, useSettingsSave } from './shared'
import styles from './settings.module.scss'

export interface ProxySettingsProps {
  data: RelayConfigResponse
  onSaved: () => Promise<unknown>
  onReload: () => void
}

const readNumber = (value: number | string, fallback: number): number => {
  const parsed = typeof value === 'number' ? value : Number(value)
  return Number.isFinite(parsed) ? Math.trunc(parsed) : fallback
}

function applyProxy(
  platform: 'youtube' | 'twitch' | 'niconico',
  result: ProxyPatchResult,
  patch: RelayUpdateConfigRequest,
  changed: RelayConfigFormKey[],
): void {
  if (result.status !== 'patch') return
  if (platform === 'youtube') {
    if (result.clear) {
      patch.clear_youtube_proxy = true
      changed.push('clear_youtube_proxy')
    } else if (result.value !== undefined) {
      patch.youtube_proxy = result.value
      changed.push('youtube_proxy')
      if (result.keep) patch.youtube_proxy_keep_password = true
    }
    return
  }
  if (platform === 'twitch') {
    if (result.clear) {
      patch.clear_twitch_proxy = true
      changed.push('clear_twitch_proxy')
    } else if (result.value !== undefined) {
      patch.twitch_proxy = result.value
      changed.push('twitch_proxy')
      if (result.keep) patch.twitch_proxy_keep_password = true
    }
    return
  }
  if (result.clear) {
    patch.clear_niconico_proxy = true
    changed.push('clear_niconico_proxy')
  } else if (result.value !== undefined) {
    patch.niconico_proxy = result.value
    changed.push('niconico_proxy')
    if (result.keep) patch.niconico_proxy_keep_password = true
  }
}

export default function ProxySettings({ data, onSaved, onReload }: ProxySettingsProps) {
  const [youtubeProxy, setYoutubeProxy] = useState(data.youtube.proxy)
  const [twitchProxy, setTwitchProxy] = useState(data.twitch.proxy)
  const [niconicoProxy, setNiconicoProxy] = useState(data.niconico.proxy)
  const [twitchRegion, setTwitchRegion] = useState(data.twitch.proxy_region)
  const [cookiesBrowser, setCookiesBrowser] = useState(data.youtube.cookies_from_browser ?? '')
  const [denoPath, setDenoPath] = useState(data.youtube.deno_path ?? '')
  const [websubUrl, setWebsubUrl] = useState(data.youtube_websub_callback_url)
  const [websubPort, setWebsubPort] = useState(data.youtube_websub_port)
  const [sessionCheck, setSessionCheck] = useState(data.niconico.session_check_enabled)
  const state = useSettingsSave()

  const youtubeResult = buildProxyPatch(
    youtubeProxy,
    data.youtube.proxy,
    data.youtube.proxy_configured,
  )
  const twitchResult = buildProxyPatch(twitchProxy, data.twitch.proxy, data.twitch.proxy_configured)
  const niconicoResult = buildProxyPatch(
    niconicoProxy,
    data.niconico.proxy,
    data.niconico.proxy_configured,
  )

  const patch: RelayUpdateConfigRequest = {}
  const changed: RelayConfigFormKey[] = []
  applyProxy('youtube', youtubeResult, patch, changed)
  applyProxy('twitch', twitchResult, patch, changed)
  applyProxy('niconico', niconicoResult, patch, changed)

  if (twitchRegion.trim() !== data.twitch.proxy_region) {
    patch.twitch_proxy_region = twitchRegion.trim()
    changed.push('twitch_proxy_region')
  }
  if (cookiesBrowser.trim() !== (data.youtube.cookies_from_browser ?? '').trim()) {
    patch.youtube_cookies_from_browser = cookiesBrowser.trim()
    changed.push('youtube_cookies_from_browser')
  }
  if (denoPath.trim() !== (data.youtube.deno_path ?? '').trim()) {
    patch.youtube_deno_path = denoPath.trim()
    changed.push('youtube_deno_path')
  }
  if (websubUrl.trim() !== data.youtube_websub_callback_url.trim()) {
    patch.youtube_websub_callback_url = websubUrl.trim()
    changed.push('youtube_websub_callback_url')
  }
  if (websubPort !== data.youtube_websub_port) {
    patch.youtube_websub_port = websubPort
    changed.push('youtube_websub_port')
  }
  if (sessionCheck !== data.niconico.session_check_enabled) {
    patch.niconico_session_check_enabled = sessionCheck
    changed.push('niconico_session_check_enabled')
  }

  const invalid = [youtubeResult, twitchResult, niconicoResult].find(
    (result) => result.status === 'invalid',
  )
  const dirty = changed.length > 0

  const save = async () => {
    if (!dirty) return
    if (invalid && invalid.status === 'invalid') {
      Toast.error(invalid.message)
      return
    }
    const url = websubUrl.trim()
    if (url && !/^https?:\/\//i.test(url)) {
      Toast.error('WebSub 回调地址必须是 http(s) 地址')
      return
    }
    if (websubPort < 1 || websubPort > 65535) {
      Toast.error('WebSub 回调端口需在 1 到 65535 之间')
      return
    }
    await state.run(
      () =>
        relayUpdateConfig({
          ...patch,
          expected: expectedConfigValues(data, changed),
          expected_secret_revision: data.secret_revision,
        }),
      onSaved,
    )
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />
      <SettingsCard
        title="代理与网络"
        description="三个平台的出站代理、WebSub 回调与 yt-dlp 相关设置。已保存的代理密码只以星号显示；把密码位清空后保存会沿用旧密码。"
      >
        <div className={styles.formGrid}>
          <SettingsField
            label="YouTube 代理"
            hint={
              data.youtube.proxy_configured
                ? '留空并保存表示清除代理；改地址时请把密码位清空以保留旧密码。'
                : 'http:// 或 socks5:// 地址；保存后密码只显示星号。'
            }
          >
            <Input
              value={youtubeProxy}
              onChange={setYoutubeProxy}
              placeholder="http://127.0.0.1:7890"
            />
          </SettingsField>
          <SettingsField label="Twitch 代理">
            <Input
              value={twitchProxy}
              onChange={setTwitchProxy}
              placeholder="http://127.0.0.1:7890"
            />
          </SettingsField>
          <SettingsField label="Niconico 代理">
            <Input
              value={niconicoProxy}
              onChange={setNiconicoProxy}
              placeholder="http://127.0.0.1:7890"
            />
          </SettingsField>
        </div>

        <div className={styles.formGrid}>
          <SettingsField label="Twitch 代理地区" hint="Twitch 的 proxy_region，例如 as、eu。">
            <Input value={twitchRegion} onChange={setTwitchRegion} placeholder="as" />
          </SettingsField>
          <SettingsField label="YouTube 登录信息来源浏览器" hint="交给 yt-dlp 的 --cookies-from-browser，例如 chrome。">
            <Input value={cookiesBrowser} onChange={setCookiesBrowser} placeholder="chrome" />
          </SettingsField>
          <SettingsField label="Deno 路径" hint="yt-dlp 需要 Deno 时填绝对路径；留空用系统 PATH。">
            <Input value={denoPath} onChange={setDenoPath} placeholder="/usr/bin/deno" />
          </SettingsField>
          <SettingsField label="WebSub 回调地址" hint="公开可访问的 http(s) 地址；留空表示不启用回调。">
            <Input
              value={websubUrl}
              onChange={setWebsubUrl}
              placeholder="https://example.com/v1/relay/websub/youtube"
            />
          </SettingsField>
          <SettingsField label="WebSub 回调端口" hint="1–65535。">
            <InputNumber
              value={websubPort}
              min={1}
              max={65535}
              precision={0}
              style={{ width: '100%' }}
              onNumberChange={(value) => setWebsubPort(readNumber(value, 0))}
              aria-label="WebSub 回调端口"
            />
          </SettingsField>
        </div>

        <div className={styles.checkRow}>
          <Checkbox
            checked={sessionCheck}
            onChange={(event) => setSessionCheck(event.target.checked === true)}
          >
            Niconico 会话定时校验
          </Checkbox>
        </div>

        <SaveRow
          dirty={dirty}
          saving={state.saving}
          onSave={() => {
            void save()
          }}
          onReset={() => {
            setYoutubeProxy(data.youtube.proxy)
            setTwitchProxy(data.twitch.proxy)
            setNiconicoProxy(data.niconico.proxy)
            setTwitchRegion(data.twitch.proxy_region)
            setCookiesBrowser(data.youtube.cookies_from_browser ?? '')
            setDenoPath(data.youtube.deno_path ?? '')
            setWebsubUrl(data.youtube_websub_callback_url)
            setWebsubPort(data.youtube_websub_port)
            setSessionCheck(data.niconico.session_check_enabled)
            state.clearError()
          }}
          okText="保存代理与网络"
        />
      </SettingsCard>
    </>
  )
}
