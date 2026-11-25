'use client'

import { TextArea, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayBannedKeywords,
  relayUpdateBannedKeywords,
  type RelayBannedKeywords,
  type RelayUpdateBannedKeywordsRequest,
} from '@/app/lib/relay'
import RelayError from '@/app/ui/relay/RelayError'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'
import styles from './settings.module.scss'

const { Text } = Typography

type KeywordKey = 'danmaku_banned_keywords' | 'streaming_banned_keywords'

const parseKeywords = (text: string): string[] =>
  text
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0)

function BannedKeywordsForm({
  data,
  onSaved,
  onReload,
}: {
  data: RelayBannedKeywords
  onSaved: () => Promise<unknown>
  onReload: () => void
}) {
  const [loaded, setLoaded] = useState(data)
  const [danmaku, setDanmaku] = useState(data.danmaku_banned_keywords.join('\n'))
  const [streaming, setStreaming] = useState(data.streaming_banned_keywords.join('\n'))
  const state = useSettingsSave()

  if (loaded !== data) {
    setLoaded(data)
    setDanmaku(data.danmaku_banned_keywords.join('\n'))
    setStreaming(data.streaming_banned_keywords.join('\n'))
  }

  const nextDanmaku = parseKeywords(danmaku)
  const nextStreaming = parseKeywords(streaming)
  const patch: RelayUpdateBannedKeywordsRequest = {}
  const changed: KeywordKey[] = []

  if (JSON.stringify(nextDanmaku) !== JSON.stringify(data.danmaku_banned_keywords)) {
    patch.danmaku_banned_keywords = nextDanmaku
    changed.push('danmaku_banned_keywords')
  }
  if (JSON.stringify(nextStreaming) !== JSON.stringify(data.streaming_banned_keywords)) {
    patch.streaming_banned_keywords = nextStreaming
    changed.push('streaming_banned_keywords')
  }

  const dirty = changed.length > 0

  const save = async () => {
    if (!dirty) return
    const expected: Record<string, unknown> = {}
    for (const key of changed) expected[key] = data[key]
    await state.run(async () => {
      const message = await relayUpdateBannedKeywords({ ...patch, expected })
      await onSaved()
      return message
    })
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />
      <div className={styles.formGrid}>
        <div className={styles.field}>
          <span className={styles.fieldLabel}>弹幕禁用关键词</span>
          <TextArea
            value={danmaku}
            onChange={setDanmaku}
            rows={8}
            autosize={false}
            spellCheck={false}
            placeholder="一行一个；命中后拦截弹幕"
            aria-label="弹幕禁用关键词"
          />
          <span className={styles.fieldHint}>保存时会去掉首尾空白与空行。</span>
        </div>
        <div className={styles.field}>
          <span className={styles.fieldLabel}>推流禁用关键词</span>
          <TextArea
            value={streaming}
            onChange={setStreaming}
            rows={8}
            autosize={false}
            spellCheck={false}
            placeholder="一行一个；命中后不转播该场"
            aria-label="推流禁用关键词"
          />
          <span className={styles.fieldHint}>保存时会去掉首尾空白与空行。</span>
        </div>
      </div>
      <SaveRow
        dirty={dirty}
        saving={state.saving}
        onSave={() => {
          void save()
        }}
        onReset={() => {
          setDanmaku(data.danmaku_banned_keywords.join('\n'))
          setStreaming(data.streaming_banned_keywords.join('\n'))
          state.clearError()
        }}
        okText="保存禁用关键词"
      />
    </>
  )
}

export default function BannedKeywords() {
  const banned = useSWR('relay:banned-keywords', relayBannedKeywords, { revalidateOnFocus: false })

  return (
    <SettingsCard
      title="禁用关键词"
      description="弹幕过滤与推流过滤共用 areas.json 里的两套关键词清单；只看这一页的命中情况请到实时日志。"
    >
      {banned.error ? (
        <RelayError
          error={banned.error}
          title="禁用关键词加载失败"
          onRetry={() => {
            void banned.mutate()
          }}
        />
      ) : null}
      {banned.data ? (
        <BannedKeywordsForm
          data={banned.data}
          onSaved={() => banned.mutate()}
          onReload={() => void banned.mutate()}
        />
      ) : (
        <Text type="tertiary" size="small">
          {banned.isLoading ? '加载中…' : '没有拿到关键词清单，请刷新重试。'}
        </Text>
      )}
    </SettingsCard>
  )
}
