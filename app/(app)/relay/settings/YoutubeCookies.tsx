'use client'

import { Tag, TextArea, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayClearYoutubeCookies,
  relayImportYoutubeCookies,
  relayYoutubeCookieStatus,
} from '@/app/lib/relay'
import { formatUnixSec } from '@/app/lib/relay-format'
import RelayError from '@/app/ui/relay/RelayError'
import relayStyles from '@/app/ui/relay/relay.module.scss'
import { FormDialog } from '@/app/ui/shell'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'

const { Text } = Typography

export default function YoutubeCookies() {
  const cookies = useSWR('relay:youtube-cookies', relayYoutubeCookieStatus, {
    revalidateOnFocus: false,
  })
  const [content, setContent] = useState('')
  const [confirmOpen, setConfirmOpen] = useState(false)
  const importState = useSettingsSave()
  const clearState = useSettingsSave()
  const status = cookies.data
  const dirty = content.trim().length > 0

  const save = async () => {
    if (!status || !dirty) return
    await importState.run(async () => {
      const next = await relayImportYoutubeCookies(content, status.revision)
      await cookies.mutate(next, { revalidate: false })
      setContent('')
      return 'YouTube Cookie 已加密保存'
    })
  }

  const clear = async () => {
    if (!status) return
    const ok = await clearState.run(async () => {
      const next = await relayClearYoutubeCookies(status.revision)
      await cookies.mutate(next, { revalidate: false })
      return 'YouTube Cookie 已清除'
    })
    if (ok) setConfirmOpen(false)
  }

  return (
    <>
      <SaveErrorBanner error={importState.error} onReload={() => void cookies.mutate()} />
      <SaveErrorBanner error={clearState.error} onReload={() => void cookies.mutate()} />
      <SettingsCard
        title="YouTube Cookie"
        description="Netscape 格式的 cookies.txt 文本，加密后写入存储；导入与清除都带修订号，冲突时会要求先刷新。"
        extra={
          <Tag size="small" color={status?.configured ? 'green' : 'grey'}>
            {status?.configured ? '已配置' : '未配置'}
          </Tag>
        }
      >
        {cookies.error ? (
          <RelayError
            error={cookies.error}
            title="Cookie 状态加载失败"
            onRetry={() => {
              void cookies.mutate()
            }}
          />
        ) : null}

        <dl className={relayStyles.kv}>
          <dt className={relayStyles.kvKey}>Cookie 条数</dt>
          <dd className={relayStyles.kvValue}>{status?.count ?? '—'}</dd>
          <dt className={relayStyles.kvKey}>修订号</dt>
          <dd className={relayStyles.kvValue}>{status?.revision ?? '—'}</dd>
          <dt className={relayStyles.kvKey}>更新时间</dt>
          <dd className={relayStyles.kvValue}>{formatUnixSec(status?.updated_at)}</dd>
        </dl>

        <Text type="tertiary" size="small">
          浏览器导出的 cookies.txt 直接粘贴到下面；提交后端会逐行校验 Netscape 格式，格式不对不会保存。
        </Text>

        <TextArea
          value={content}
          onChange={setContent}
          rows={8}
          autosize={false}
          spellCheck={false}
          placeholder="# Netscape HTTP Cookie File"
          aria-label="YouTube Cookie 内容"
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
          okText="导入 Cookie"
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
            清除已保存的 Cookie
          </Text>
        </div>
      </SettingsCard>

      {confirmOpen && status ? (
        <FormDialog
          title="清除 YouTube Cookie"
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
            将删除已加密保存的 {status.count} 条 Cookie（修订号 {status.revision}）。之后需要重新导入才能继续使用需要登录态的 YouTube 抓取。
          </Text>
        </FormDialog>
      ) : null}
    </>
  )
}
