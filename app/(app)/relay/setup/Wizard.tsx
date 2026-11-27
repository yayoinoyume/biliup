'use client'

import {
  Banner,
  Button,
  Checkbox,
  Input,
  InputNumber,
  Steps,
  Switch,
  Tag,
  Toast,
  Typography,
} from '@douyinfe/semi-ui'
import { useEffect, useRef, useState } from 'react'
import Link from 'next/link'
import useSWR from 'swr'
import { QRCodeSVG } from 'qrcode.react'
import {
  relayPreviewSetupFavorites,
  relayResolveYoutubeChannel,
  relaySaveSetupConfig,
  relaySetupBiliRoom,
  relaySetupLoginStatus,
  relaySetupPollLogin,
  relaySetupQrCode,
  relaySetupStatus,
} from '@/app/lib/relay'
import type {
  RelayHolodexFavoriteChannel,
  RelayPollLoginResult,
  RelayQrCode,
  RelaySetupArea,
  RelaySetupConfigRequest,
} from '@/app/lib/relay'
import type { User } from '@/app/lib/api-streamer'
import { useBiliUsers } from '@/app/lib/use-streamers'
import AreaCatalogPicker, {
  RELAY_AREA_GROUPS_KEY,
  loadRelayAreaGroups,
} from '@/app/ui/relay/AreaCatalogPicker'
import RelayError, { relayErrorMessage } from '@/app/ui/relay/RelayError'
import MaskedSecretInput from '@/app/ui/relay/MaskedSecretInput'
import styles from '@/app/ui/relay/relay.module.scss'
import pageStyles from '../components/relay-overview.module.scss'
import { RELAY_SETUP_KEY, RELAY_SETUP_LOGIN_KEY } from '../components/keys'
import { Field } from '../components/ui'

const { Text, Title } = Typography

interface WizardForm {
  room: number
  intervalSeconds: number
  autoCover: boolean
  enableDanmaku: boolean
  antiCollision: boolean
  enableLolMonitor: boolean
  holodexApiKey: string
  holodexJwt: string
  youtubeProxy: string
  twitchProxy: string
  youtubeEnabled: boolean
  youtubeName: string
  youtubeId: string
  youtubeArea: number | null
  youtubeQuality: string
  twitchEnabled: boolean
  twitchName: string
  twitchId: string
  twitchArea: number | null
  twitchQuality: string
  twitchRegion: string
  niconicoEnabled: boolean
  niconicoName: string
  niconicoId: string
  niconicoArea: number | null
}

const INITIAL_FORM: WizardForm = {
  room: 0,
  intervalSeconds: 60,
  autoCover: true,
  enableDanmaku: true,
  antiCollision: false,
  enableLolMonitor: false,
  holodexApiKey: '',
  holodexJwt: '',
  youtubeProxy: '',
  twitchProxy: '',
  youtubeEnabled: false,
  youtubeName: '',
  youtubeId: '',
  youtubeArea: null,
  youtubeQuality: 'best',
  twitchEnabled: false,
  twitchName: '',
  twitchId: '',
  twitchArea: null,
  twitchQuality: 'best',
  twitchRegion: 'as',
  niconicoEnabled: false,
  niconicoName: '',
  niconicoId: '',
  niconicoArea: null,
}

export default function Wizard() {
  const [form, setForm] = useState<WizardForm>(INITIAL_FORM)
  const [step, setStep] = useState(0)
  const [qr, setQr] = useState<RelayQrCode | null>(null)
  const [loadingQr, setLoadingQr] = useState(false)
  const [polling, setPolling] = useState(false)
  const [pollResult, setPollResult] = useState<RelayPollLoginResult | null>(null)
  const [resolving, setResolving] = useState(false)
  const [readingRoom, setReadingRoom] = useState(false)
  const [previewing, setPreviewing] = useState(false)
  const [favoriteOptions, setFavoriteOptions] = useState<RelayHolodexFavoriteChannel[]>([])
  const [favoritePicks, setFavoritePicks] = useState<string[]>([])
  const [selectedAreas, setSelectedAreas] = useState<RelaySetupArea[]>([])
  const [areaPick, setAreaPick] = useState<number | null>(null)
  const [saving, setSaving] = useState(false)
  const [savedMessage, setSavedMessage] = useState<string | null>(null)
  const pollTimer = useRef<number | null>(null)
  const aliveRef = useRef(true)
  const pollSeqRef = useRef(0)
  const setup = useSWR(RELAY_SETUP_KEY, relaySetupStatus, { revalidateOnFocus: false })
  const login = useSWR(RELAY_SETUP_LOGIN_KEY, relaySetupLoginStatus, { revalidateOnFocus: false })
  const areaGroups = useSWR(RELAY_AREA_GROUPS_KEY, loadRelayAreaGroups, {
    revalidateOnFocus: false,
  })
  const { biliUsers } = useBiliUsers()

  useEffect(() => {
    aliveRef.current = true
    return () => {
      aliveRef.current = false
      pollSeqRef.current += 1
      if (pollTimer.current !== null) {
        window.clearTimeout(pollTimer.current)
        pollTimer.current = null
      }
    }
  }, [])

  const update = <K extends keyof WizardForm>(key: K, value: WizardForm[K]) => {
    setForm((previous) => ({ ...previous, [key]: value }))
  }

  const fillRoomFromAccount = async () => {
    if (readingRoom) return
    setReadingRoom(true)
    try {
      const room = await relaySetupBiliRoom()
      if (room !== null) update('room', room)
      else Toast.info('账号还没有直播间，请手动填写房间号')
    } catch (error) {
      Toast.error(relayErrorMessage(error))
    } finally {
      setReadingRoom(false)
    }
  }

  const stopPolling = () => {
    pollSeqRef.current += 1
    if (pollTimer.current !== null) {
      window.clearTimeout(pollTimer.current)
      pollTimer.current = null
    }
    setPolling(false)
  }

  const startPolling = (authCode: string, seq: number) => {
    pollTimer.current = window.setTimeout(() => {
      pollTimer.current = null
      if (!aliveRef.current || seq !== pollSeqRef.current) return
      void relaySetupPollLogin(authCode)
        .then((result) => {
          if (!aliveRef.current || seq !== pollSeqRef.current) return
          setPollResult(result)
          if (result.status === 'waiting') {
            startPolling(authCode, seq)
            return
          }
          stopPolling()
          if (result.status === 'success') {
            Toast.success(result.message || 'B 站账号登录成功')
            void login.mutate()
            void fillRoomFromAccount()
          }
        })
        .catch((error) => {
          if (!aliveRef.current || seq !== pollSeqRef.current) return
          stopPolling()
          Toast.error(relayErrorMessage(error))
        })
    }, 2000)
  }

  const requestQrCode = async () => {
    if (loadingQr || polling) return
    const seq = pollSeqRef.current + 1
    pollSeqRef.current = seq
    if (pollTimer.current !== null) {
      window.clearTimeout(pollTimer.current)
      pollTimer.current = null
    }
    setLoadingQr(true)
    try {
      const next = await relaySetupQrCode()
      if (!aliveRef.current || seq !== pollSeqRef.current) return
      setQr(next)
      setPollResult(null)
      setPolling(true)
      startPolling(next.auth_code, seq)
    } catch (error) {
      if (!aliveRef.current || seq !== pollSeqRef.current) return
      Toast.error(relayErrorMessage(error))
    } finally {
      if (aliveRef.current && seq === pollSeqRef.current) setLoadingQr(false)
    }
  }

  const resolveYoutube = async () => {
    const input = form.youtubeId.trim()
    if (!input || resolving) return
    setResolving(true)
    try {
      const channelId = await relayResolveYoutubeChannel(input)
      update('youtubeId', channelId)
      Toast.success('已解析为 YouTube 频道 ID')
    } catch (error) {
      Toast.error(relayErrorMessage(error))
    } finally {
      setResolving(false)
    }
  }

  const addSelectedArea = () => {
    if (areaPick === null) return
    if (selectedAreas.some((area) => area.id === areaPick)) {
      Toast.info('这个分区已经在名单里了')
      return
    }
    const found = (areaGroups.data ?? [])
      .flatMap((group) => group.areas)
      .find((area) => area.id === areaPick)
    if (!found) {
      Toast.error('没有找到这个分区的名称，请刷新分区列表')
      return
    }
    setSelectedAreas([
      ...selectedAreas,
      { id: found.id, name: found.name, parent_name: found.parent_name },
    ])
    setAreaPick(null)
  }

  const previewFavorites = async () => {
    if (previewing) return
    setPreviewing(true)
    try {
      const list = await relayPreviewSetupFavorites({
        api_key: form.holodexApiKey.trim(),
        jwt: form.holodexJwt.trim(),
      })
      setFavoriteOptions(list)
      setFavoritePicks([])
      if (list.length === 0) {
        Toast.info('收藏夹里没有可导入的 YouTube 频道')
      }
    } catch (error) {
      Toast.error(relayErrorMessage(error))
    } finally {
      setPreviewing(false)
    }
  }

  const toggleFavorite = (id: string, checked: boolean) => {
    setFavoritePicks((previous) =>
      checked ? [...previous, id] : previous.filter((value) => value !== id),
    )
  }

  const validatePlatforms = (): string | null => {
    const platforms = [
      {
        enabled: form.youtubeEnabled,
        name: form.youtubeName,
        id: form.youtubeId,
        label: 'YouTube',
      },
      { enabled: form.twitchEnabled, name: form.twitchName, id: form.twitchId, label: 'Twitch' },
      {
        enabled: form.niconicoEnabled,
        name: form.niconicoName,
        id: form.niconicoId,
        label: 'Niconico',
      },
    ]
    for (const platform of platforms) {
      if (platform.enabled && (!platform.name.trim() || !platform.id.trim())) {
        return platform.label + ' 启用时必须填写频道名与频道 ID'
      }
    }
    return null
  }

  const buildPayload = (): RelaySetupConfigRequest => {
    const payload: RelaySetupConfigRequest = {
      room: form.room,
      auto_cover: form.autoCover,
      enable_danmaku_command: form.enableDanmaku,
      interval: form.intervalSeconds,
      anti_collision: form.antiCollision,
      enable_lol_monitor: form.enableLolMonitor,
      youtube_enable_monitor: form.youtubeEnabled,
      twitch_enable_monitor: form.twitchEnabled,
      niconico_enable_monitor: form.niconicoEnabled,
      selected_areas: selectedAreas,
      selected_youtube_channels: favoriteOptions.filter((channel) =>
        favoritePicks.includes(channel.id),
      ),
    }
    if (form.youtubeName.trim()) payload.youtube_channel_name = form.youtubeName.trim()
    if (form.youtubeId.trim()) payload.youtube_channel_id = form.youtubeId.trim()
    if (form.youtubeArea !== null) payload.youtube_area_v2 = form.youtubeArea
    if (form.youtubeQuality.trim()) payload.youtube_quality = form.youtubeQuality.trim()
    if (form.youtubeProxy.trim()) payload.youtube_proxy = form.youtubeProxy.trim()
    if (form.twitchName.trim()) payload.twitch_channel_name = form.twitchName.trim()
    if (form.twitchId.trim()) payload.twitch_channel_id = form.twitchId.trim()
    if (form.twitchArea !== null) payload.twitch_area_v2 = form.twitchArea
    if (form.twitchQuality.trim()) payload.twitch_quality = form.twitchQuality.trim()
    if (form.twitchRegion.trim()) payload.twitch_proxy_region = form.twitchRegion.trim()
    if (form.twitchProxy.trim()) payload.twitch_proxy = form.twitchProxy.trim()
    if (form.niconicoName.trim()) payload.niconico_channel_name = form.niconicoName.trim()
    if (form.niconicoId.trim()) payload.niconico_channel_id = form.niconicoId.trim()
    if (form.niconicoArea !== null) payload.niconico_area_v2 = form.niconicoArea
    if (form.holodexApiKey.trim()) payload.holodex_api_key = form.holodexApiKey.trim()
    if (form.holodexJwt.trim()) payload.holodex_jwt = form.holodexJwt.trim()
    return payload
  }

  const save = async () => {
    if (form.room <= 0) {
      Toast.error('请填写直播间号')
      setStep(0)
      return
    }
    if (form.intervalSeconds <= 0) {
      Toast.error('检查间隔必须大于 0')
      setStep(1)
      return
    }
    const platformError = validatePlatforms()
    if (platformError) {
      Toast.error(platformError)
      return
    }
    if (saving) return
    setSaving(true)
    setSavedMessage(null)
    try {
      const message = await relaySaveSetupConfig(buildPayload())
      setSavedMessage(message ?? '首次设置已保存')
      Toast.success(message ?? '首次设置已保存')
      void setup.mutate()
    } catch (error) {
      Toast.error(relayErrorMessage(error))
    } finally {
      setSaving(false)
    }
  }

  const nextStep = () => {
    if (step === 0 && form.room <= 0) {
      Toast.error('请先填写直播间号')
      return
    }
    if (step === 1 && form.intervalSeconds <= 0) {
      Toast.error('检查间隔必须大于 0')
      return
    }
    setStep(Math.min(step + 1, 2))
  }

  const loginCheckedIn = login.data?.logged_in === true
  const hostAccounts = biliUsers as User[]
  const hostAccount = hostAccounts[0] ?? null
  const hostAccountLabel = hostAccount
    ? hostAccount.name?.trim() || `账号 #${hostAccount.id}`
    : ''

  return (
    <div className={styles.stack}>
      <section className={styles.card}>
        <Steps current={step} type="basic" size="small">
          <Steps.Step title="B 站账号" description="账号与房间号" />
          <Steps.Step title="基础参数" description="检查间隔与密钥" />
          <Steps.Step title="频道与分区" description="监控目标与初始名单" />
        </Steps>
        <div style={{ marginTop: 12 }}>
          <Text type="tertiary" size="small">
            {setup.isLoading
              ? '正在检查首次设置…'
              : setup.data?.needs_setup
                ? '还需要完成首次设置'
                : '首次设置已完成'}
          </Text>
        </div>
        {setup.error ? (
          <RelayError
            error={setup.error}
            title="配置检查读取失败"
            onRetry={() => {
              void setup.mutate()
            }}
          />
        ) : null}
        {setup.data?.storage_error ? (
          <Banner
            className={styles.banner}
            type="danger"
            closeIcon={null}
            title="数据存储不可用"
            description={setup.data.storage_error}
          />
        ) : null}
        {setup.data && !setup.data.needs_setup && !setup.data.storage_error ? (
          <Banner
            className={styles.banner}
            type="warning"
            closeIcon={null}
            title="首次设置已经完成"
            description="继续保存会覆盖房间号、检查间隔与平台监控开关。只调整日常参数时请前往转播设置页。"
          />
        ) : null}
      </section>

      <section className={styles.card}>
        {step === 0 ? (
          <div className={pageStyles.stepBody}>
            <div className={styles.sectionTitle}>
              <Title heading={6} style={{ margin: 0 }}>
                第 1 步：B 站账号与直播间
              </Title>
              <Tag
                size="small"
                color={
                  hostAccount
                    ? 'green'
                    : login.error
                      ? 'grey'
                      : loginCheckedIn
                        ? 'green'
                        : 'orange'
                }
              >
                {hostAccount
                  ? '已选择 B 站账号'
                  : login.error
                    ? '登录状态未知'
                    : loginCheckedIn
                      ? 'B 站账号已登录'
                      : '尚未登录'}
              </Tag>
            </div>
            {hostAccount ? (
              <Banner
                className={styles.banner}
                type="success"
                closeIcon={null}
                title={`将使用 B 站账号：${hostAccountLabel}`}
                description={
                  hostAccounts.length > 1
                    ? `保存后转播会使用这个 B 站账号；房间号请手动填写。保存后可在转播设置里更换（账号中心共 ${hostAccounts.length} 个，默认用第一个）。`
                    : '保存后转播会使用这个 B 站账号；房间号请手动填写。保存后可在转播设置里更换。'
                }
              />
            ) : login.error ? (
              <RelayError
                error={login.error}
                title="B 站登录状态读取失败"
                onRetry={() => {
                  void login.mutate()
                }}
              />
            ) : (
              <Banner
                className={styles.banner}
                type={loginCheckedIn ? 'success' : 'info'}
                closeIcon={null}
                title={loginCheckedIn ? 'B 站账号已登录' : '用 B 站扫码登录'}
                description={
                  loginCheckedIn
                    ? '登录信息已保存到加密库，下面的房间号可以从账号自动读取。'
                    : '扫码登录后会自动保存登录信息，便于后续开播与弹幕指令。也可以先手填房间号继续，稍后在设置页补登录。'
                }
              />
            )}
            <div className={styles.toolbar}>
              {hostAccount ? null : (
                <Button
                  theme="solid"
                  type="primary"
                  loading={loadingQr}
                  disabled={polling}
                  onClick={() => void requestQrCode()}
                >
                  {polling ? '等待扫码…' : qr ? '重新获取二维码' : '用 B 站扫码登录'}
                </Button>
              )}
              <Button
                loading={readingRoom}
                disabled={!loginCheckedIn}
                onClick={() => void fillRoomFromAccount()}
              >
                从账号读取房间号
              </Button>
              <Link href="/relay/settings" className={pageStyles.textLink}>
                恢复备份 / 打开转播设置
              </Link>
            </div>
            {qr ? (
              <div className={pageStyles.qrBox}>
                <QRCodeSVG value={qr.qr_url} size={200} level="M" marginSize={2} />
              </div>
            ) : null}
            {pollResult ? (
              <Text type={pollResult.status === 'success' ? 'success' : 'tertiary'} size="small">
                {pollResult.status === 'success'
                  ? '扫码登录成功'
                  : pollResult.status === 'expired'
                    ? '二维码已过期，请重新获取'
                    : pollResult.message}
              </Text>
            ) : null}
            <Field
              label="直播间号"
              hint={
                hostAccount && !loginCheckedIn
                  ? '保存后会使用所选的 B 站账号；房间号请手动填写。'
                  : '登录后可以从账号自动读取；也可以直接填写。'
              }
            >
              <InputNumber
                value={form.room > 0 ? form.room : undefined}
                min={1}
                precision={0}
                placeholder="例如 123456"
                style={{ width: '100%' }}
                onNumberChange={(value) => update('room', Number(value) || 0)}
              />
            </Field>
          </div>
        ) : null}

        {step === 1 ? (
          <div className={pageStyles.stepBody}>
            <div className={styles.sectionTitle}>
              <Title heading={6} style={{ margin: 0 }}>
                第 2 步：基础参数
              </Title>
            </div>
            <Banner
              className={styles.banner}
              type="info"
              closeIcon={null}
              title="后台登录与权限统一由 B 站账号管理"
              description="这里只设置转播服务自己的参数，不再单独维护另一套登录密码。网页登录、角色和权限请到「登录用户与权限」管理。"
            />
            <div className={pageStyles.formGrid}>
              <Field label="检查间隔（秒）" hint="多久检查一次直播状态，默认 60 秒。">
                <InputNumber
                  value={form.intervalSeconds > 0 ? form.intervalSeconds : undefined}
                  min={5}
                  precision={0}
                  suffix="秒"
                  style={{ width: '100%' }}
                  onNumberChange={(value) => update('intervalSeconds', Number(value) || 0)}
                />
              </Field>
              <Field label="自动封面">
                <Switch checked={form.autoCover} onChange={(checked) => update('autoCover', checked)} />
              </Field>
              <Field label="弹幕指令">
                <Switch
                  checked={form.enableDanmaku}
                  onChange={(checked) => update('enableDanmaku', checked)}
                />
              </Field>
              <Field label="防撞车">
                <Switch
                  checked={form.antiCollision}
                  onChange={(checked) => update('antiCollision', checked)}
                />
              </Field>
              <Field label="LOL 监控">
                <Switch
                  checked={form.enableLolMonitor}
                  onChange={(checked) => update('enableLolMonitor', checked)}
                />
              </Field>
            </div>
            <MaskedSecretInput
              label="Holodex API Key"
              value={form.holodexApiKey}
              onChange={(value) => update('holodexApiKey', value)}
              configured={false}
              help="用于公开的 Holodex 密钥；留空可稍后在设置页配置。"
            />
            <MaskedSecretInput
              label="Holodex JWT"
              value={form.holodexJwt}
              onChange={(value) => update('holodexJwt', value)}
              configured={false}
              multiline
              rows={3}
              help="收藏列表与账号相关功能需要登录令牌（JWT）；留空可稍后在设置页配置。"
            />
            <div className={pageStyles.formGrid}>
              <Field label="YouTube 代理" hint="形如 http://127.0.0.1:7890；留空表示直连。">
                <Input
                  value={form.youtubeProxy}
                  onChange={(value) => update('youtubeProxy', value)}
                  placeholder="http://..."
                />
              </Field>
              <Field label="Twitch 代理" hint="留空表示直连。">
                <Input
                  value={form.twitchProxy}
                  onChange={(value) => update('twitchProxy', value)}
                  placeholder="http://..."
                />
              </Field>
            </div>
            <Text type="tertiary" size="small">
              YouTube API Key 不能在首次设置里保存，
              <Link href="/relay/settings" className={pageStyles.textLink}>
                请到转播设置页配置
              </Link>
              。
            </Text>
          </div>
        ) : null}

        {step === 2 ? (
          <div className={pageStyles.stepBody}>
            <div className={styles.sectionTitle}>
              <Title heading={6} style={{ margin: 0 }}>
                第 3 步：频道与分区
              </Title>
            </div>
            <div className={styles.grid}>
              <section className={styles.card}>
                <div className={styles.sectionTitle}>
                  <Title heading={6} style={{ margin: 0 }}>
                    YouTube
                  </Title>
                  <Switch
                    checked={form.youtubeEnabled}
                    onChange={(checked) => update('youtubeEnabled', checked)}
                  />
                </div>
                <div className={styles.stack}>
                  <Field label="频道名">
                    <Input
                      value={form.youtubeName}
                      onChange={(value) => update('youtubeName', value)}
                      placeholder="显示用频道名"
                    />
                  </Field>
                  <Field label="频道 ID / 链接" hint="支持 @handle 与频道网址，点解析会转成 UC 开头的 ID。">
                    <div className={pageStyles.inlineEdit}>
                      <Input
                        value={form.youtubeId}
                        onChange={(value) => update('youtubeId', value)}
                        placeholder="UC... 或 @handle"
                      />
                      <Button loading={resolving} onClick={() => void resolveYoutube()}>
                        解析
                      </Button>
                    </div>
                  </Field>
                  <Field label="默认分区">
                    <AreaCatalogPicker
                      value={form.youtubeArea}
                      onChange={(areaId) => update('youtubeArea', areaId)}
                    />
                  </Field>
                  <Field label="画质">
                    <Input
                      value={form.youtubeQuality}
                      onChange={(value) => update('youtubeQuality', value)}
                      placeholder="best"
                    />
                  </Field>
                </div>
              </section>

              <section className={styles.card}>
                <div className={styles.sectionTitle}>
                  <Title heading={6} style={{ margin: 0 }}>
                    Twitch
                  </Title>
                  <Switch
                    checked={form.twitchEnabled}
                    onChange={(checked) => update('twitchEnabled', checked)}
                  />
                </div>
                <div className={styles.stack}>
                  <Field label="频道名">
                    <Input
                      value={form.twitchName}
                      onChange={(value) => update('twitchName', value)}
                      placeholder="显示用频道名"
                    />
                  </Field>
                  <Field label="频道 ID">
                    <Input
                      value={form.twitchId}
                      onChange={(value) => update('twitchId', value)}
                      placeholder="Twitch 登录名"
                    />
                  </Field>
                  <Field label="默认分区">
                    <AreaCatalogPicker
                      value={form.twitchArea}
                      onChange={(areaId) => update('twitchArea', areaId)}
                    />
                  </Field>
                  <Field label="画质">
                    <Input
                      value={form.twitchQuality}
                      onChange={(value) => update('twitchQuality', value)}
                      placeholder="best"
                    />
                  </Field>
                  <Field label="代理区域">
                    <Input
                      value={form.twitchRegion}
                      onChange={(value) => update('twitchRegion', value)}
                      placeholder="as"
                    />
                  </Field>
                </div>
              </section>

              <section className={styles.card}>
                <div className={styles.sectionTitle}>
                  <Title heading={6} style={{ margin: 0 }}>
                    Niconico
                  </Title>
                  <Switch
                    checked={form.niconicoEnabled}
                    onChange={(checked) => update('niconicoEnabled', checked)}
                  />
                </div>
                <div className={styles.stack}>
                  <Field label="频道名">
                    <Input
                      value={form.niconicoName}
                      onChange={(value) => update('niconicoName', value)}
                      placeholder="显示用频道名"
                    />
                  </Field>
                  <Field label="频道 ID">
                    <Input
                      value={form.niconicoId}
                      onChange={(value) => update('niconicoId', value)}
                      placeholder="频道 ID"
                    />
                  </Field>
                  <Field label="默认分区">
                    <AreaCatalogPicker
                      value={form.niconicoArea}
                      onChange={(areaId) => update('niconicoArea', areaId)}
                    />
                  </Field>
                </div>
              </section>
            </div>

            <section className={styles.card}>
              <div className={styles.sectionTitle}>
                <Title heading={6} style={{ margin: 0 }}>
                  额外分区名单
                </Title>
                <Text type="tertiary" size="small">
                  保存时会合并进本机的分区名单
                </Text>
              </div>
              <div className={pageStyles.formGrid}>
                <AreaCatalogPicker value={areaPick} onChange={setAreaPick} />
                <div className={pageStyles.inlineEdit}>
                  <Button onClick={addSelectedArea} disabled={areaPick === null}>
                    添加到名单
                  </Button>
                </div>
              </div>
              <div className={pageStyles.chipRow} style={{ marginTop: 10 }}>
                {selectedAreas.length === 0 ? (
                  <Text type="tertiary" size="small">
                    还没有额外分区。
                  </Text>
                ) : (
                  selectedAreas.map((area) => (
                    <Tag
                      key={area.id}
                      size="small"
                      closable
                      onClose={() => {
                        setSelectedAreas(selectedAreas.filter((item) => item.id !== area.id))
                      }}
                    >
                      {area.name}（{area.id}）
                    </Tag>
                  ))
                )}
              </div>
            </section>

            <section className={styles.card}>
              <div className={styles.sectionTitle}>
                <Title heading={6} style={{ margin: 0 }}>
                  Holodex 收藏预览
                </Title>
                <Button loading={previewing} onClick={() => void previewFavorites()}>
                  预览收藏
                </Button>
              </div>
              <Text type="tertiary" size="small">
                使用第 2 步填写的 Holodex API Key / JWT 读取收藏；勾选的频道会写进频道名单。
              </Text>
              {favoriteOptions.length > 0 ? (
                <div className={pageStyles.checkList}>
                  {favoriteOptions.map((channel) => (
                    <Checkbox
                      key={channel.id}
                      checked={favoritePicks.includes(channel.id)}
                      onChange={(event) => toggleFavorite(channel.id, Boolean(event.target.checked))}
                    >
                      {channel.name}（{channel.id}）
                    </Checkbox>
                  ))}
                </div>
              ) : null}
            </section>

            {savedMessage ? (
              <Banner
                className={styles.banner}
                type="success"
                closeIcon={null}
                title="首次设置已保存"
                description={
                  <div className={styles.errorBody}>
                    <Text>{savedMessage}</Text>
                    <div className={styles.errorActions}>
                      <Link href="/relay" className={pageStyles.textLink}>
                        返回转播总览
                      </Link>
                    </div>
                  </div>
                }
              />
            ) : null}
          </div>
        ) : null}

        <div className={pageStyles.wizardFooter}>
          <div className={pageStyles.wizardFooterActions}>
            {step > 0 ? (
              <Button onClick={() => setStep(step - 1)} disabled={saving}>
                上一步
              </Button>
            ) : null}
            {step < 2 ? (
              <Button theme="solid" type="primary" onClick={nextStep}>
                下一步
              </Button>
            ) : (
              <Button theme="solid" type="primary" loading={saving} onClick={() => void save()}>
                保存首次设置
              </Button>
            )}
          </div>
          <Text type="tertiary" size="small">
            第 {step + 1} / 3 步
          </Text>
        </div>
      </section>
    </div>
  )
}
