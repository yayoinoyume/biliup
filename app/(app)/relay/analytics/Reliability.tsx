'use client'

/**
 * 可靠性：干净结束、计划内结束与计入的失败次数，外加失败原因分类。
 * `reliability.reasons` 的 `class` 是后端诊断串，前端原样展示，不自己编映射。
 */

import type { RelayHistorySummary } from '@/app/lib/relay'
import relay from '@/app/ui/relay/relay.module.scss'
import styles from './analytics.module.scss'

export interface ReliabilityProps {
  reliability: RelayHistorySummary['reliability']
}

export default function Reliability({ reliability }: ReliabilityProps) {
  const reasons = [...reliability.reasons].sort((a, b) => b.count - a.count)
  const total = reasons.reduce((sum, reason) => sum + reason.count, 0)

  return (
    <div className={relay.stack}>
      <div className={relay.statGrid}>
        <div className={relay.stat}>
          <span className={relay.statLabel}>干净结束</span>
          <span className={relay.statValue}>{reliability.clean}</span>
        </div>
        <div className={relay.stat}>
          <span className={relay.statLabel}>计划内结束</span>
          <span className={relay.statValue}>{reliability.planned}</span>
        </div>
        <div className={relay.stat}>
          <span className={relay.statLabel}>计入失败</span>
          <span className={relay.statValue}>{reliability.counted_failures}</span>
        </div>
      </div>
      {reasons.length === 0 ? (
        <p className={relay.muted}>这段区间没有失败原因记录。</p>
      ) : (
        <ul className={styles.reasons}>
          {reasons.map((reason) => (
            <li key={reason.class}>
              <span className={relay.mono}>{reason.class}</span>
              <span className={styles.reasonCount}>
                {reason.count}
                <span className={relay.muted}>
                  {' '}
                  ({total > 0 ? Math.round((reason.count / total) * 100) : 0}%)
                </span>
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
