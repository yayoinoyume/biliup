'use client'

/**
 * 在线状态分布条：每个桶一段，颜色只分三档（全程在线 / 有缺口 / 完全离线）。
 * 与日历一样，桶的时间由后端按 `tz_offset_min` 对齐，前端只负责画。
 */

import type { RelayHistorySummaryBucket } from '@/app/lib/relay'
import { formatDurationSec, formatUnixSec } from '@/app/lib/relay-format'
import relay from '@/app/ui/relay/relay.module.scss'
import { bucketLiveSecs } from './useHistory'
import styles from './analytics.module.scss'

export interface StripProps {
  buckets: RelayHistorySummaryBucket[]
  /** 一个桶的秒数，用来算在线比例。 */
  bucketSecs: number
}

function live(bucket: RelayHistorySummaryBucket, bucketSecs: number): number {
  if (bucketSecs <= 0) return 0
  return Math.min(1, bucketLiveSecs(bucket.live_secs) / bucketSecs)
}

export default function Strip({ buckets, bucketSecs }: StripProps) {
  const total = buckets.reduce((sum, bucket) => sum + bucketLiveSecs(bucket.live_secs), 0)
  const full = buckets.filter((bucket) => live(bucket, bucketSecs) >= 0.95).length

  if (buckets.length === 0) {
    return (
      <p className={relay.muted}>
        这段区间没有分桶数据。
      </p>
    )
  }

  return (
    <div className={styles.stripWrap}>
      <div className={styles.strip} role="img" aria-label={`在线状态分布，共 ${buckets.length} 个时间桶`}>
        {buckets.map((bucket) => {
          const ratio = live(bucket, bucketSecs)
          const state = ratio >= 0.95 ? 'online' : ratio > 0 ? 'partial' : 'offline'
          return (
            <span
              key={bucket.start}
              className={
                state === 'online'
                  ? styles.stripOnline
                  : state === 'partial'
                    ? styles.stripPartial
                    : styles.stripOffline
              }
              title={`${formatUnixSec(bucket.start)} 起：推流 ${formatDurationSec(
                bucketLiveSecs(bucket.live_secs),
              )}，${bucket.sessions} 场，${bucket.failures} 次失败`}
            />
          )
        })}
      </div>
      <div className={styles.stripLegend}>
        <span><i className={styles.dotOnline} />全程在线 {full}</span>
        <span><i className={styles.dotPartial} />有过缺口</span>
        <span><i className={styles.dotOffline} />离线</span>
        <span className={relay.muted}>合计推流 {formatDurationSec(total)}</span>
      </div>
    </div>
  )
}
