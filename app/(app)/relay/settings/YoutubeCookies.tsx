'use client'

import { Button, Tag, TextArea, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayClearYoutubeCookies,
  relayImportYoutubeCookies,
  relaySelectYoutubeAccount,
  relayYoutubeAccounts,
  relayYoutubeCookieStatus,
} from '@/app/lib/relay'
import { formatUnixSec } from '@/app/lib/relay-format'
import RelayError from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import { FormDialog } from '@/app/ui/shell'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'
import styles from './settings.module.scss'

const { Text } = Typography

export default function YoutubeCookies() {
  const accounts = useSWR('relay:youtube-accounts', relayYoutubeAccounts, {
    revalidateOnFocus: false,
  })
  const cookies = useSWR('relay:youtube-cookies', relayYoutubeCookieStatus, {
    revalidateOnFocus: false,
  })
  const [content, setContent] = useState('')
  const [confirmOpen, setConfirmOpen] = useState(false)
  const importState = useSettingsSave()
  const clearState = useSettingsSave()
  const selectState = useSettingsSave()
  const status = cookies.data
  const list = accounts.data
  const selectedId = list?.selected_id ?? null
  const selected = list?.accounts.find((account) => account.id === selectedId) ?? null
  const dirty = content.trim().length > 0

  const choose = async (id: string | null) => {
    if (!list || selectedId === id) return
    await selectState.run(async () => {
      const next = await relaySelectYoutubeAccount(id, list.revision)
      await accounts.mutate(next, { revalidate: false })
      return id ? '已切换 YouTube 账号' : '已改回旧版登录信息（备用）'
    })
  }

  const save = async () => {
    if (!status || !dirty) return
    await importState.run(async () => {
      const next = await relayImportYoutubeCookies(content, status.revision)
      await cookies.mutate(next, { revalidate: false })
      setContent('')
      return 'YouTube 登录信息已加密保存'
    })
  }

  const clear = async () => {
    if (!status) return
    const ok = await clearState.run(async () => {
      const next = await relayClearYoutubeCookies(status.revision)
      await cookies.mutate(next, { revalidate: false })
      return 'YouTube 登录信息已清除'
    })
    if (ok) setConfirmOpen(false)
  }

  return (
    <>
      <SettingsCard
        title="YouTube 账号"
        description="账号中心里的 YouTube 登录信息。选中一个账号后抓取就用它的登录信息；没选时才用下面的旧版单份登录信息。这里只负责选用，增删在账号中心。"
        extra={
          <Tag size="small" color={selected ? 'green' : 'grey'}>
            {selected ? selected.label : '未选择（使用旧版登录信息）'}
          </Tag>
        }
      >
        {accounts.error ? (
          <RelayError
            error={accounts.error}
            title="YouTube 账号列表加载失败"
            onRetry={() => {
              void accounts.mutate()
            }}
          />
        ) : null}
        <SaveErrorBanner error={selectState.error} onReload={() => void accounts.mutate()} />

        {list && list.accounts.length > 0 ? (
          <ul className={styles.list}>
            {list.accounts.map((account) => {
              const current = account.id === selectedId
              return (
                <li key={account.id} className={styles.listRow}>
                  <div className={styles.listMain}>
                    <Text strong>{account.label}</Text>
                    <Tag size="small">
                      {account.source_type === 'browser' ? '浏览器' : 'cookies.txt'}
                    </Tag>
                    <Text type="tertiary" size="small">
                      {account.source_type === 'browser'
                        ? [account.browser, account.profile].filter(Boolean).join(' / ') || '浏览器'
                        : account.cookie_count + ' 条登录信息'}
                    </Text>
                    <Text type="tertiary" size="small">
                      更新于 {formatUnixSec(account.updated_at)}
                    </Text>
                  </div>
                  <Button
                    size="small"
                    theme={current ? 'solid' : 'light'}
                    loading={selectState.saving}
                    disabled={current || selectState.saving}
                    onClick={() => {
                      void choose(account.id)
                    }}
                  >
                    {current ? '当前使用' : '设为当前'}
                  </Button>
                </li>
              )
            })}
          </ul>
        ) : null}

        {list && list.accounts.length === 0 ? (
          <Text type="tertiary" size="small">
            账号中心还没有 YouTube 账号。可以先去那里添加，或者继续用下面的旧版登录信息。
          </Text>
        ) : null}

        {selected ? (
          <div className={styles.actions}>
            <Button
              size="small"
              loading={selectState.saving}
              onClick={() => {
                void choose(null)
              }}
            >
              改回旧版登录信息（备用）
            </Button>
          </div>
        ) : null}
      </SettingsCard>

      <SaveErrorBanner error={importState.error} onReload={() => void cookies.mutate()} />
      <SaveErrorBanner error={clearState.error} onReload={() => void cookies.mutate()} />
      <SettingsCard
        title="旧版 YouTube 登录信息（备用）"
        description="浏览器导出的 Netscape 格式 cookies.txt 文本，加密后写入存储。只有未选中任何账号时才会用到它；导入与清除都带版本号，冲突时会要求先刷新。"
        extra={
          <Tag size="small" color={status?.configured ? 'green' : 'grey'}>
            {status?.configured ? '已配置' : '未配置'}
          </Tag>
        }
      >
        {cookies.error ? (
          <RelayError
            error={cookies.error}
            title="登录信息状态加载失败"
            onRetry={() => {
              void cookies.mutate()
            }}
          />
        ) : null}

        <dl className={relayStyles.kv}>
          <dt className={relayStyles.kvKey}>登录信息条数</dt>
          <dd className={relayStyles.kvValue}>{status?.count ?? '—'}</dd>
          <dt className={relayStyles.kvKey}>版本号</dt>
          <dd className={relayStyles.kvValue}>{status?.revision ?? '—'}</dd>
          <dt className={relayStyles.kvKey}>更新时间</dt>
          <dd className={relayStyles.kvValue}>{formatUnixSec(status?.updated_at)}</dd>
        </dl>

        <Text type="tertiary" size="small">
          {selected
            ? '现在用的是账号「' + selected.label + '」，这份旧版登录信息不会被读取。'
            : '浏览器导出的 cookies.txt 直接粘贴到下面；提交后端会逐行校验 Netscape 格式，格式不对不会保存。'}
        </Text>

        <TextArea
          value={content}
          onChange={setContent}
          rows={8}
          autosize={false}
          spellCheck={false}
          placeholder="# Netscape HTTP Cookie File"
          aria-label="YouTube 登录信息内容"
        />

        <SaveRow
          dirty={dirty}
          saving={importState.saving}
          onSave={() => {
            void save()
          }}
          onReset={() => {
            setContent('')
            importState.clearError()
          }}
          okText="导入登录信息"
          disabled={!status}
        />
        <div>
          <Text
            type="danger"
            size="small"
            link={status?.configured === true}
            onClick={() => {
              if (!status?.configured) return
              clearState.clearError()
              setConfirmOpen(true)
            }}
          >
            清除已保存的登录信息
          </Text>
        </div>
      </SettingsCard>

      {confirmOpen && status ? (
        <FormDialog
          title="清除 YouTube 登录信息"
          visible={confirmOpen}
          size="sm"
          okText="确认清除"
          okType="danger"
          onOk={() => clear()}
          onCancel={() => setConfirmOpen(false)}
        >
          {clearState.error ? (
            <SaveErrorBanner
              error={clearState.error}
              onReload={() => {
                setConfirmOpen(false)
                void cookies.mutate()
              }}
            />
          ) : null}
          <Text>
            将删除已加密保存的 {status.count} 条旧登录信息（版本号 {status.revision}）。选中账号不受影响；清除后如果没有选中账号，需要重新导入才能继续使用需要登录的 YouTube 抓取。
          </Text>
        </FormDialog>
      ) : null}
    </>
  )
}
