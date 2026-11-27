'use client'

import { useRef, useState } from 'react'
import useSWR, { mutate } from 'swr'
import useSWRMutation from 'swr/mutation'
import {
  Banner,
  Button,
  Empty,
  Form,
  List,
  Popconfirm,
  Radio,
  RadioGroup,
  Spin,
  TabPane,
  Tabs,
  Tag,
  Toast,
  Typography,
} from '@douyinfe/semi-ui'
import { IconPlusCircle, IconUserGroup } from '@douyinfe/semi-icons'
import type { FormApi } from '@douyinfe/semi-ui/lib/es/form'
import { useRouter } from 'next/navigation'
import PageHeader from '../components/PageHeader'
import dc from '@/app/ui/data-card.module.scss'
import styles from './page.module.scss'
import AvatarCard from '@/app/ui/AvatarCard'
import { AddAccountDialog } from '@/app/ui/UserList'
import { requestDelete, sendRequest } from '@/app/lib/api-streamer'
import { useBiliUsers } from '@/app/lib/use-streamers'
import { useMe } from '@/app/lib/use-me'
import { isRelayApiError } from '@/app/lib/relay'
import { formatUnixSec } from '@/app/lib/relay-format'
import { relayErrorMessage } from '@/app/ui/relay/RelayError'
import { FormDialog } from '@/app/ui/shell'
import {
  youtubeAccounts,
  youtubeAddAccount,
  youtubeDeleteAccount,
  youtubeSelectAccount,
  type YoutubeAccount,
  type YoutubeAccountSource,
  type YoutubeAccountsSnapshot,
} from '@/app/lib/youtube-accounts'

const { Text } = Typography

const BROWSERS = [
  { value: 'chrome', label: 'Chrome' },
  { value: 'firefox', label: 'Firefox' },
  { value: 'edge', label: 'Edge' },
  { value: 'brave', label: 'Brave' },
]

function errorText(error: unknown): string {
  const raw = (error as { message?: string })?.message ?? String(error)
  try {
    const parsed = JSON.parse(raw) as { error?: string; message?: string }
    return parsed.error ?? parsed.message ?? raw
  } catch {
    return raw
  }
}

function sourceLabel(account: YoutubeAccount) {
  if (account.source_type === 'cookies') return 'cookies.txt'
  const browser = BROWSERS.find((item) => item.value === account.browser)?.label ?? account.browser ?? '浏览器'
  return account.profile ? browser + '（' + account.profile + '）' : browser
}

function BiliAccountsPanel() {
  const { can } = useMe()
  const canRead = can('template.edit')
  const { biliUsers, isLoading, isError } = useBiliUsers()
  const { trigger: addUser } = useSWRMutation('/v1/users', sendRequest)
  const { trigger: deleteUser } = useSWRMutation('/v1/users', requestDelete)
  const [adding, setAdding] = useState(false)

  const add = async (value: string) => {
    try {
      await addUser({ value })
      setAdding(false)
      Toast.success('B 站账号已添加')
      return true
    } catch (error) {
      Toast.error({ content: errorText(error), duration: 6 })
      return false
    }
  }

  const remove = async (id: number) => {
    try {
      await deleteUser(id)
      Toast.success('B 站账号已删除')
    } catch (error) {
      Toast.error({ content: errorText(error), duration: 6 })
    }
  }

  if (!canRead) {
    return (
      <Empty
        title="无法读取 B 站账号"
        description="查看账号列表需要「编辑投稿模板」权限，请联系超级管理员调整角色。"
      />
    )
  }

  return (
    <div className={styles.panel}>
      <div className={styles.toolbar}>
        <Text type="tertiary" size="small">
          {isLoading ? '正在读取账号…' : '共 ' + biliUsers.length + ' 个账号'}
        </Text>
        <Button icon={<IconPlusCircle />} theme="solid" onClick={() => setAdding(true)}>
          添加 B 站账号
        </Button>
      </div>
      {isLoading ? (
        <div className={styles.center}>
          <Spin size="large" />
        </div>
      ) : isError ? (
        <div className={styles.center}>
          <Empty title="加载失败" description={errorText(isError)} />
          <Button onClick={() => void mutate('/v1/users')} style={{ marginTop: 12 }}>
            重试
          </Button>
        </div>
      ) : biliUsers.length === 0 ? (
        <div className={styles.center}>
          <Empty title="还没有 B 站账号" description="扫码登录，或填写 biliup login 导出的登录信息文件路径。">
            <Button icon={<IconPlusCircle />} theme="solid" onClick={() => setAdding(true)}>
              添加账号
            </Button>
          </Empty>
        </div>
      ) : (
        <div className={dc.card}>
          <List
            dataSource={biliUsers}
            split={false}
            size="small"
            renderItem={(item) => (
              <AvatarCard
                url={item.face}
                abbr={item.name}
                label={item.name}
                value={item.value}
                onRemove={() => remove(item.id)}
              />
            )}
          />
        </div>
      )}
      {adding ? <AddAccountDialog onClose={() => setAdding(false)} onAdd={add} /> : null}
    </div>
  )
}

function YoutubeAccountsPanel() {
  const router = useRouter()
  const accounts = useSWR('relay:youtube-accounts', youtubeAccounts, { revalidateOnFocus: false })
  const [addOpen, setAddOpen] = useState(false)
  const [busy, setBusy] = useState<string | null>(null)
  const [conflict, setConflict] = useState(false)

  const save = async (next: YoutubeAccountsSnapshot | null) => {
    if (next) await accounts.mutate(next, { revalidate: false })
    else await accounts.mutate()
  }

  const run = async (
    key: string,
    task: () => Promise<YoutubeAccountsSnapshot | null>,
    message: string,
  ) => {
    setBusy(key)
    try {
      const next = await task()
      await save(next)
      setConflict(false)
      Toast.success(message)
      return true
    } catch (error) {
      if (isRelayApiError(error) && error.status === 409) {
        setConflict(true)
        Toast.error('账号列表已在别处修改，请刷新后重试')
      } else {
        Toast.error({ content: relayErrorMessage(error), duration: 6 })
      }
      return false
    } finally {
      setBusy(null)
    }
  }

  const select = (id: string | null) => {
    const snapshot = accounts.data
    if (!snapshot) return Promise.resolve(false)
    const message = id ? '已设为当前 YouTube 账号' : '已取消选中 YouTube 账号'
    return run('select:' + (id ?? 'none'), () => youtubeSelectAccount(id, snapshot.revision), message)
  }

  const remove = (account: YoutubeAccount) => {
    const snapshot = accounts.data
    if (!snapshot) return Promise.resolve(false)
    return run(
      'delete:' + account.id,
      () => youtubeDeleteAccount(account.id, snapshot.revision),
      '已删除 ' + account.label,
    )
  }

  const create = async (label: string, source: YoutubeAccountSource) => {
    const snapshot = accounts.data
    if (!snapshot) return false
    const ok = await run(
      'create',
      () => youtubeAddAccount({ label, expected_revision: snapshot.revision, source }),
      'YouTube 账号已添加',
    )
    if (ok) setAddOpen(false)
    return ok
  }

  const refresh = async () => {
    setConflict(false)
    await accounts.mutate()
  }

  if (accounts.error) {
    const notConfigured =
      isRelayApiError(accounts.error) &&
      (accounts.error.status === 412 || accounts.error.code === 'not_configured')

    if (notConfigured) {
      return (
        <div className={styles.panel}>
          <Banner
            type="warning"
            closeIcon={null}
            title="转播服务尚未配置"
            description="YouTube 账号保存在转播账号存储里；先完成首次设置，再回来管理账号。"
          />
          <Text type="tertiary" size="small">
            {relayErrorMessage(accounts.error)}
          </Text>
          <div className={styles.toolbarActions}>
            <Button theme="solid" onClick={() => router.push('/relay/setup')}>
              前往首次设置
            </Button>
            <Button onClick={() => void accounts.mutate()}>重试</Button>
          </div>
        </div>
      )
    }

    return (
      <div className={styles.panel}>
        <div className={styles.center}>
          <Empty title="账号列表加载失败" description={relayErrorMessage(accounts.error)} />
          <Button onClick={() => void accounts.mutate()} style={{ marginTop: 12 }}>
            重试
          </Button>
        </div>
      </div>
    )
  }

  const data = accounts.data
  const selected = data?.selected_id ?? null

  return (
    <div className={styles.panel}>
      {conflict ? (
        <Banner
          type="warning"
          closeIcon={null}
          title="账号列表已在别处修改"
          description={
            <span>
              请刷新列表后重试刚才的操作。
              <Button size="small" theme="light" onClick={() => void refresh()} style={{ marginLeft: 8 }}>
                刷新列表
              </Button>
            </span>
          }
        />
      ) : null}
      <div className={styles.toolbar}>
        <Text type="tertiary" size="small">
          {data
            ? '共 ' + data.accounts.length + ' 个账号' + (selected ? '，已选中一个' : '，未选中账号')
            : '正在读取账号…'}
        </Text>
        <div className={styles.toolbarActions}>
          {selected ? (
            <Button theme="light" loading={busy === 'select:none'} onClick={() => void select(null)}>
              取消选中
            </Button>
          ) : null}
          <Button icon={<IconPlusCircle />} theme="solid" onClick={() => setAddOpen(true)}>
            添加 YouTube 账号
          </Button>
        </div>
      </div>
      {!data ? (
        <div className={styles.center}>
          <Spin size="large" />
        </div>
      ) : data.accounts.length === 0 ? (
        <div className={styles.center}>
          <Empty
            title="还没有 YouTube 账号"
            description="添加浏览器登录信息或 Netscape 格式的 cookies.txt，转播就能用它访问 YouTube。"
          >
            <Button icon={<IconPlusCircle />} theme="solid" onClick={() => setAddOpen(true)}>
              添加账号
            </Button>
          </Empty>
        </div>
      ) : (
        <div className={dc.card}>
          <ul className={styles.accountList}>
            {data.accounts.map((account) => (
              <li key={account.id} className={styles.accountRow}>
                <div className={styles.accountInfo}>
                  <div className={styles.accountTitle}>
                    <span className={styles.accountLabel}>{account.label}</span>
                    {account.id === data.selected_id ? (
                      <Tag size="small" color="green">
                        当前使用
                      </Tag>
                    ) : null}
                  </div>
                  <div className={styles.accountMeta}>
                    <span>{sourceLabel(account)}</span>
                    <span>登录信息 {account.cookie_count} 条</span>
                    <span>更新于 {formatUnixSec(account.updated_at)}</span>
                  </div>
                </div>
                <div className={styles.accountActions}>
                  <Button
                    size="small"
                    theme="light"
                    disabled={account.id === data.selected_id || busy !== null}
                    loading={busy === 'select:' + account.id}
                    onClick={() => void select(account.id)}
                  >
                    设为当前
                  </Button>
                  <Popconfirm
                    title={'删除 ' + account.label + '？'}
                    content="删除后这份登录信息不再可用；如果转播正在使用它，会退回未选中状态。"
                    okType="danger"
                    onConfirm={() => remove(account)}
                  >
                    <Button
                      size="small"
                      type="danger"
                      theme="borderless"
                      loading={busy === 'delete:' + account.id}
                    >
                      删除
                    </Button>
                  </Popconfirm>
                </div>
              </li>
            ))}
          </ul>
        </div>
      )}
      {addOpen && data ? (
        <AddYoutubeAccountDialog onClose={() => setAddOpen(false)} onCreate={create} />
      ) : null}
    </div>
  )
}

type YoutubeSourceType = 'browser' | 'cookies'

interface YoutubeAccountFormValues {
  label?: string
  browser?: string
  profile?: string
  content?: string
}

function AddYoutubeAccountDialog({
  onClose,
  onCreate,
}: {
  onClose: () => void
  onCreate: (label: string, source: YoutubeAccountSource) => Promise<boolean>
}) {
  const [type, setType] = useState<YoutubeSourceType>('browser')
  const [saving, setSaving] = useState(false)
  const api = useRef<FormApi<YoutubeAccountFormValues>>(undefined)

  const submit = async (values: YoutubeAccountFormValues) => {
    const label = values.label?.trim()
    if (!label) return
    const profile = values.profile?.trim()
    const source: YoutubeAccountSource =
      type === 'browser'
        ? { type: 'browser', browser: values.browser ?? 'chrome', ...(profile ? { profile } : {}) }
        : { type: 'cookies', content: values.content ?? '' }
    setSaving(true)
    try {
      await onCreate(label, source)
    } finally {
      setSaving(false)
    }
  }

  return (
    <FormDialog
      title="添加 YouTube 账号"
      size="md"
      onCancel={onClose}
      okText="添加"
      onOk={() => api.current?.submitForm()}
      confirmLoading={saving}
    >
      <RadioGroup
        type="button"
        value={type}
        onChange={(event) => setType(event.target.value as YoutubeSourceType)}
        aria-label="YouTube 账号来源"
      >
        <Radio value="browser">浏览器</Radio>
        <Radio value="cookies">cookies.txt</Radio>
      </RadioGroup>
      <Form<YoutubeAccountFormValues>
        getFormApi={(formApi) => (api.current = formApi)}
        onSubmit={submit}
        initValues={{ browser: 'chrome' }}
        labelPosition="top"
        style={{ marginTop: 12 }}
      >
        <Form.Input
          field="label"
          label="名称"
          placeholder="例如 主用 YouTube 账号"
          maxLength={64}
          rules={[
            { required: true, message: '给这个账号起个名字' },
            {
              validator: (_rule: unknown, value: string) => !!value && value.trim() === value,
              message: '首尾不能有空格',
            },
          ]}
        />
        {type === 'browser' ? (
          <>
            <Form.Select
              field="browser"
              label="浏览器"
              style={{ width: '100%' }}
              optionList={BROWSERS}
              rules={[{ required: true, message: '选择浏览器' }]}
              extraText="只在本机读取浏览器登录信息，浏览器需与转播服务在同一台机器。"
            />
            <Form.Input
              field="profile"
              label="浏览器配置（可选）"
              placeholder="Profile 2"
              extraText="多账号浏览器可以填配置目录名；留空使用默认配置。"
            />
          </>
        ) : (
          <Form.TextArea
            field="content"
            label="cookies.txt 内容"
            rows={8}
            autosize={false}
            spellCheck={false}
            placeholder="# Netscape HTTP Cookie File"
            rules={[{ required: true, message: '粘贴 Netscape 格式的 cookies.txt 内容' }]}
            extraText="由服务器访问 YouTube 使用；内容加密保存，接口不回传。"
          />
        )}
      </Form>
    </FormDialog>
  )
}

export default function AccountsPage() {
  const { can } = useMe()
  const [tab, setTab] = useState<'bili' | 'youtube'>('bili')
  const showYoutube = can('relay.config')
  const activeTab = tab === 'youtube' && !showYoutube ? 'bili' : tab

  return (
    <>
      <PageHeader
        icon={<IconUserGroup size="large" />}
        title="账号中心"
        description="B 站账号与 YouTube 账号集中管理；YouTube 账号保存在转播账号存储里。"
      />
      <div className={dc.content}>
        <Tabs
          type="line"
          activeKey={activeTab}
          onChange={(key) => setTab(key === 'youtube' ? 'youtube' : 'bili')}
        >
          <TabPane itemKey="bili" tab="B 站账号">
            <BiliAccountsPanel />
          </TabPane>
          {showYoutube ? (
            <TabPane itemKey="youtube" tab="YouTube 账号">
              <YoutubeAccountsPanel />
            </TabPane>
          ) : null}
        </Tabs>
      </div>
    </>
  )
}
