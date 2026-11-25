'use client'

import { useState } from 'react'
import { Button, Input, Toast, Typography } from '@douyinfe/semi-ui'
import { relayResolveYoutubeChannel } from '@/app/lib/relay'
import { relayErrorMessage } from '@/app/ui/relay/RelayError'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text } = Typography

const CHANNEL_ID = /^UC[\w-]{22}$/

const PATH_SUFFIXES = new Set([
  'videos',
  'streams',
  'featured',
  'shorts',
  'about',
  'playlists',
  'playlist',
  'community',
  'channels',
  'live',
  'posts',
  'watch',
  'results',
  'feed',
])

function firstSegment(value: string): string {
  return value.split(/[/?#]/)[0].trim()
}

export function youtubeNameFromInput(input: string): string {
  const value = input.trim()
  if (!value || CHANNEL_ID.test(value)) return ''
  if (value.startsWith('@')) return firstSegment(value.slice(1))
  if (!/[/.]/.test(value)) return value
  let path = value
  try {
    const url = new URL(/^[a-z][a-z0-9+.-]*:\/\//i.test(value) ? value : `https://${value}`)
    path = url.pathname
  } catch {
    return firstSegment(value)
  }
  const segments = path.split('/').filter(Boolean)
  if (segments.some((segment, index) => segment === 'channel' && CHANNEL_ID.test(segments[index + 1] ?? ''))) {
    return ''
  }
  const handle = segments.find((segment) => segment.startsWith('@'))
  if (handle) return handle.slice(1)
  const last = segments[segments.length - 1] ?? ''
  const previous = segments[segments.length - 2] ?? ''
  const name = PATH_SUFFIXES.has(last.toLowerCase()) ? previous : last
  if (!name || CHANNEL_ID.test(name)) return ''
  return name.startsWith('@') ? name.slice(1) : name
}

export default function YoutubeResolver({
  disabled = false,
  onResolved,
}: {
  disabled?: boolean
  onResolved: (channelId: string, suggestedName: string) => void
}) {
  const [input, setInput] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const resolve = async () => {
    const value = input.trim()
    if (!value) {
      setError('请填写频道链接、@handle 或频道 ID')
      return
    }
    setBusy(true)
    setError(null)
    try {
      const channelId = await relayResolveYoutubeChannel(value)
      onResolved(channelId, youtubeNameFromInput(value))
      Toast.success(`已解析出频道 ID：${channelId}`)
    } catch (failure) {
      setError(relayErrorMessage(failure))
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className={styles.field}>
      <div className={styles.fieldHead}>
        <span className={styles.fieldLabel}>从链接或 @handle 解析 YouTube 频道</span>
      </div>
      <div className={styles.toolbar}>
        <Input
          value={input}
          onChange={setInput}
          onEnterPress={() => {
            void resolve()
          }}
          placeholder="https://www.youtube.com/@handle 或 @handle"
          disabled={disabled || busy}
          style={{ flex: 1, minWidth: 0 }}
        />
        <Button
          loading={busy}
          disabled={disabled}
          onClick={() => {
            void resolve()
          }}
        >
          解析
        </Button>
      </div>
      {error ? (
        <Text type="danger" size="small">
          {error}
        </Text>
      ) : (
        <Text type="tertiary" size="small">
          解析结果填入上面的频道 ID；名称留空时用 handle 充当。
        </Text>
      )}
    </div>
  )
}
