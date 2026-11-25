'use client'

/**
 * 实时日志（`/relay/logs`）。
 *
 * `/v1/relay/logs` 只回整份内存环形日志（`{success, logs}`，`logs` 是按 \n 拼起来的纯文本），
 * 没有增量与游标，所以这里每 5 秒整份拉一次；级别在客户端逐行解析。
 * 清屏只记录「截断点」——保留可见行里的最后一行原文，下次刷新按它定位，
 * 于是环形缓冲淘汰旧行也不会把清屏后的新行藏起来。
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button, Checkbox, Input, Spin, Switch, Typography } from '@douyinfe/semi-ui'
import { IconTerminal } from '@douyinfe/semi-icons'
import PageHeader from '../../components/PageHeader'
import PermissionGate from '@/app/ui/relay/PermissionGate'
import RelayError from '@/app/ui/relay/RelayError'
import { relayLogs } from '@/app/lib/relay'
import relay from '@/app/ui/relay/relay.module.scss'
import styles from './logs.module.scss'

const { Text } = Typography

/** 后端契约写死的轮询间隔。 */
const POLL_MS = 5000

/** 一次最多渲染这么多行：环形缓冲可能存着几万行，全量 DOM 会让页面卡死。 */
const MAX_RENDER = 2000

const LEVELS = ['ERROR', 'WARN', 'INFO', 'DEBUG'] as const
type Level = (typeof LEVELS)[number]

const LEVEL_PATTERN = /(^|\s)(TRACE|DEBUG|INFO|WARN|ERROR)\b/

interface Line {
  key: number
  text: string
  level: Level
}

/** 认不出的级别按 INFO 处理：既不会被默认过滤掉，也不会被染成危险色。 */
function levelOf(text: string): Level {
  const hit = LEVEL_PATTERN.exec(text)
  const found = hit?.[2]
  if (found === 'ERROR' || found === 'WARN' || found === 'INFO' || found === 'DEBUG') return found
  return 'INFO'
}

export default function RelayLogsPage() {
  const [lines, setLines] = useState<string[]>([])
  const [error, setError] = useState<unknown>(null)
  const [loading, setLoading] = useState(true)
  const [paused, setPaused] = useState(false)
  const [updatedAt, setUpdatedAt] = useState<number | null>(null)
  const [levels, setLevels] = useState<Level[]>([...LEVELS])
  const [search, setSearch] = useState('')
  const [autoScroll, setAutoScroll] = useState(true)
  const [cutAnchor, setCutAnchor] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)

  const seqRef = useRef(0)
  const viewRef = useRef<HTMLDivElement>(null)

  const load = useCallback(async () => {
    const seq = (seqRef.current += 1)
    try {
      const snapshot = await relayLogs()
      if (seq !== seqRef.current) return
      setLines(snapshot.logs ? snapshot.logs.split('\n') : [])
      setError(null)
      setUpdatedAt(Date.now())
    } catch (failure) {
      if (seq !== seqRef.current) return
      setError(failure)
    } finally {
      if (seq === seqRef.current) setLoading(false)
    }
  }, [])

  useEffect(() => {
    if (paused) return
    let closed = false
    const tick = () => {
      if (!closed) void load()
    }
    // 首次拉取放进微任务：effect 体内不直接触发状态更新
    queueMicrotask(tick)
    const timer = setInterval(tick, POLL_MS)
    return () => {
      closed = true
      clearInterval(timer)
    }
  }, [paused, load])

  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), 2000)
    return () => clearTimeout(timer)
  }, [copied])

  /** 清屏后要跳过的那段：按截断点的原文在最新快照里重新定位。 */
  const visible = useMemo(() => {
    if (cutAnchor === null) return lines
    const index = lines.lastIndexOf(cutAnchor)
    return index >= 0 ? lines.slice(index + 1) : lines
  }, [lines, cutAnchor])

  const parsed = useMemo<Line[]>(
    () => visible.map((text, index) => ({ key: index, text, level: levelOf(text) })),
    [visible],
  )

  const filtered = useMemo(() => {
    const keyword = search.trim().toLowerCase()
    return parsed.filter(
      (line) =>
        levels.includes(line.level) &&
        (keyword === '' || line.text.toLowerCase().includes(keyword)),
    )
  }, [parsed, levels, search])

  const rendered = filtered.slice(-MAX_RENDER)
  const renderedCount = rendered.length

  useEffect(() => {
    if (!autoScroll) return
    const view = viewRef.current
    if (view) view.scrollTop = view.scrollHeight
  }, [autoScroll, renderedCount])

  const toggleLevel = (level: Level) => {
    setLevels((current) =>
      current.includes(level) ? current.filter((item) => item !== level) : [...current, level],
    )
  }

  const copyVisible = async () => {
    try {
      await navigator.clipboard.writeText(rendered.map((line) => line.text).join('\n'))
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return (
    <div className={relay.content}>
      <PageHeader
        title="实时日志"
        description="转播底座的内存环形日志"
        icon={<IconTerminal />}
        breadcrumbs={[{ href: '/relay', label: '直播转播' }]}
        actions={
          <div className={relay.toolbar}>
            <span className={styles.switchLine}>
              <Text type="tertiary" size="small">
                每 5 秒轮询
              </Text>
              <Switch size="small" checked={!paused} onChange={(value) => setPaused(!value)} />
            </span>
            <Button theme="light" onClick={() => void load()}>
              立即刷新
            </Button>
          </div>
        }
      />

      <PermissionGate perm="relay.view">
        {error ? (
          <RelayError
            error={error}
            onRetry={() => {
              void load()
            }}
          />
        ) : null}

        <section className={relay.card}>
          <div className={relay.sectionTitle}>
            <div className={styles.filters}>
              {LEVELS.map((level) => (
                <Checkbox
                  key={level}
                  checked={levels.includes(level)}
                  onChange={() => toggleLevel(level)}
                >
                  <span className={styles[levelClass(level)]}>
                    {level}
                  </span>
                </Checkbox>
              ))}
            </div>
            <div className={styles.actions}>
              <Input
                value={search}
                onChange={setSearch}
                placeholder="关键词"
                showClear
                style={{ width: 180 }}
              />
              <Button theme="light" onClick={() => void copyVisible()}>
                {copied ? '已复制' : '复制可见'}
              </Button>
              <Button theme="light" onClick={() => setCutAnchor(lines[lines.length - 1] ?? null)}>
                清屏
              </Button>
              {cutAnchor !== null ? (
                <Button theme="light" onClick={() => setCutAnchor(null)}>
                  恢复全部
                </Button>
              ) : null}
            </div>
          </div>

          <div className={styles.meta}>
            <span>
              总 {lines.length} 行 · 筛选 {filtered.length} 行
              {filtered.length > MAX_RENDER ? ` · 只渲染最后 ${MAX_RENDER} 行` : ''}
            </span>
            <span className={styles.metaRight}>
              <Checkbox checked={autoScroll} onChange={() => setAutoScroll((value) => !value)}>
                自动滚到底部
              </Checkbox>
              <Text type="tertiary" size="small">
                {updatedAt === null ? '尚未刷新' : `更新于 ${new Date(updatedAt).toLocaleTimeString('zh-CN')}`}
              </Text>
            </span>
          </div>

          <div className={styles.view} ref={viewRef}>
            {loading && lines.length === 0 ? (
              <Spin size="small" />
            ) : rendered.length === 0 ? (
              <Text type="tertiary" size="small">
                {lines.length === 0 ? '暂时没有日志输出。' : '没有符合当前筛选的日志行。'}
              </Text>
            ) : (
              rendered.map((line) => (
                <div key={line.key} className={`${styles.line} ${styles[levelClass(line.level)]}`}>
                  {line.text || ' '}
                </div>
              ))
            )}
          </div>
        </section>
      </PermissionGate>
    </div>
  )
}

function levelClass(level: Level): 'error' | 'warn' | 'info' | 'debug' {
  if (level === 'ERROR') return 'error'
  if (level === 'WARN') return 'warn'
  if (level === 'DEBUG') return 'debug'
  return 'info'
}
