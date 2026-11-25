'use client'

import { TextArea, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayGetPlayerFilter,
  relaySavePlayerFilter,
  type RelayPlayerFilter,
} from '@/app/lib/relay'
import RelayError from '@/app/ui/relay/RelayError'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'

const { Text } = Typography

const EMPTY_FILTER: RelayPlayerFilter = { content: '', revision: 0 }

function PlayerFilterForm({
  data,
  creating,
  onSaved,
  onReload,
}: {
  data: RelayPlayerFilter
  creating: boolean
  onSaved: () => Promise<unknown>
  onReload: () => void
}) {
  const [loaded, setLoaded] = useState(data)
  const [content, setContent] = useState(data.content)
  const state = useSettingsSave()

  if (loaded !== data) {
    setLoaded(data)
    setContent(data.content)
  }

  const dirty = content !== data.content

  const save = async () => {
    if (!dirty) return
    await state.run(async () => {
      const message = await relaySavePlayerFilter({
        content,
        expected_revision: data.revision,
      })
      await onSaved()
      return message
    })
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />
      <Text type="tertiary" size="small">
        {creating
          ? '过滤词文档尚不存在：保存会以修订号 0 创建，若别的窗口刚创建过会返回冲突，点「重新加载配置」后重试。'
          : '每行一条过滤词；保存时带当前修订号 ' + data.revision + '，冲突时请先重新加载。'}
      </Text>
      <TextArea
        value={content}
        onChange={setContent}
        rows={10}
        autosize={false}
        spellCheck={false}
        placeholder="一行一条过滤词"
        aria-label="播放器过滤词"
      />
      <SaveRow
        dirty={dirty}
        saving={state.saving}
        onSave={() => {
          void save()
        }}
        onReset={() => {
          setContent(data.content)
          state.clearError()
        }}
        okText="保存过滤词"
      />
    </>
  )
}

export default function PlayerFilter() {
  const filter = useSWR('relay:player-filter', relayGetPlayerFilter, { revalidateOnFocus: false })
  const data = filter.data ?? EMPTY_FILTER

  return (
    <SettingsCard
      title="播放器过滤词"
      description="存于加密存储的 invalid_words.txt，前端只按修订号做比较交换，不会盲目覆盖别人的修改。"
    >
      {filter.error ? (
        <RelayError
          error={filter.error}
          title="过滤词加载失败"
          onRetry={() => {
            void filter.mutate()
          }}
        />
      ) : null}
      <PlayerFilterForm
        data={data}
        creating={!filter.data}
        onSaved={() => filter.mutate()}
        onReload={() => void filter.mutate()}
      />
      {filter.isLoading ? (
        <Text type="tertiary" size="small">
          加载中…
        </Text>
      ) : null}
    </SettingsCard>
  )
}
