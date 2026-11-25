'use client'

/**
 * 每桶推流时长：用 CSS 柱而不是再拉一个图表库，桶数由后端决定（hour / 6h / day）。
 * 只有首尾与「最高那根」标出时刻，避免 90 根柱子把标签压成一团。
 */

import type { RelayHistorySummaryBucket } from '@/app/lib/relay'
import { formatDurationSec, formatUnixSec } from '@/app/lib/relay-format'
import relay from '@/app/ui/relay/relay.module.scss'
import styles from './analytics.module.scss'

export interface HoursProps {
  buckets: RelayHistorySummaryBucket[]
}

export default function Hours({ buckets }: HoursProps) {
  const peak = buckets.reduce((max, bucket) => Math.max(max, bucket.live_secs), 0)
  const tallest = buckets.findIndex((bucket) => bucket.live_secs === peak)

  if (buckets.length === 0) {
    return <p className={relay.muted}>这段区间没有推流记录。</p>
  }

  return (
    <div className={styles.barsWrap}>
      <div className={styles.bars} role="img" aria-label="每个时间桶的推流时长">
        {buckets.map((bucket) => (
          <span
            key={bucket.start}
            className={styles.barSlot}
            title={`${formatUnixSec(bucket.start)}：${formatDurationSec(bucket.live_secs)}，${bucket.sessions} 场`}
          >
            <span
              className={styles.barFill}
              style={{ height: peak > 0 ? `${Math.round((bucket.live_secs / peak) * 100)}%` : '0%' }}
            />
          </span>
        ))}
      </div>
      <div className={styles.barsAxis}>
        <span>{formatUnixSec(buckets[0]?.start)}</span>
        {tallest >= 0 ? (
          <span className={relay.muted}>
            峰值 {formatDurationSec(peak)} @ {formatUnixSec(buckets[tallest]?.start)}
          </span>
        ) : null}
        <span>{formatUnixSec(buckets[buckets.length - 1]?.start)}</span>
      </div>
    </div>
  )
}
