'use client'

import { Button, TextArea, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import {
  relayGetPlayerFilter,
  relaySavePlayerFilter,
  type RelayPlayerFilter,
} from '@/app/lib/relay'
import RelayError from '@/app/ui/relay/RelayError'
import { SaveErrorBanner, SaveRow, SettingsCard, useSettingsSave } from './shared'
import styles from './settings.module.scss'

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
    await state.run(
      () =>
        relaySavePlayerFilter({
          content,
          expected_revision: data.revision,
        }),
      onSaved,
    )
  }

  return (
    <>
      <SaveErrorBanner error={state.error} onReload={onReload} />
      <Text type="tertiary" size="small">
        {creating
          ? '将新建过滤词（版本号 0）；若别的窗口刚创建过会提示冲突，请先刷新再编辑。'
          : '每行一条过滤词；保存时带当前版本号 ' + data.revision + '，冲突时请先重新加载。'}
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
  const [creating, setCreating] = useState(false)
  const reload = () => {
    void filter.mutate()
  }

  return (
    <SettingsCard
      title="播放器过滤词"
      description="过滤词保存在服务端；页面按版本号避免互相覆盖。"
    >
      {filter.error ? (
        <RelayError error={filter.error} title="过滤词加载失败" onRetry={reload} />
      ) : null}

      {filter.data ? (
        <PlayerFilterForm
          data={filter.data}
          creating={false}
          onSaved={() => filter.mutate()}
          onReload={reload}
        />
      ) : null}

      {!filter.data && !filter.error && filter.isLoading ? (
        <Text type="tertiary" size="small">
          加载中…
        </Text>
      ) : null}

      {!filter.data && filter.error && creating ? (
        <PlayerFilterForm
          data={EMPTY_FILTER}
          creating
          onSaved={() => filter.mutate()}
          onReload={reload}
        />
      ) : null}

      {!filter.data && filter.error && !creating ? (
        <>
          <div className={styles.actions}>
            <Button onClick={() => setCreating(true)}>新建过滤词（版本号 0）</Button>
            <Text type="tertiary" size="small">
              读取失败可能是网络或权限问题；确认要新建过滤词时再继续。
            </Text>
          </div>
        </>
      ) : null}

      {!filter.data && !filter.error && !filter.isLoading ? (
        <Text type="tertiary" size="small">
          没有拿到过滤词，请刷新重试。
        </Text>
      ) : null}
    </SettingsCard>
  )
}
