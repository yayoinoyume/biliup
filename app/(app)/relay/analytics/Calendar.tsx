'use client'

/**
 * 开播日历：`summary.calendar` 固定回 91 天（今天往前 90 天），
 * 每天一个格子，颜色深浅按时长分档；点格子把场次列表收窄到当天，再点一次取消。
 *
 * 契约里 `day` 被声明成字符串，但后端发的是 Unix 秒（`query.rs` 的 `align(now, DAY, off)`，
 * 序列化成 `{"day": day}`）。这里按实际类型读，数字与 `YYYY-MM-DD` 都能认。
 */

import type { RelayHistorySummary } from '@/app/lib/relay'
import { formatDurationSec } from '@/app/lib/relay-format'
import relay from '@/app/ui/relay/relay.module.scss'
import styles from './analytics.module.scss'

export interface CalendarProps {
  calendar: RelayHistorySummary['calendar']
  /** 当前选中的当天零点（Unix 秒）；null 表示没选。 */
  selected: number | null
  onSelect: (dayStart: number | null) => void
}

const WEEKDAYS = ['一', '二', '三', '四', '五', '六', '日']

/** 后端实际发 Unix 秒；契约写的字符串也一并认掉。 */
function daySeconds(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value
  if (typeof value === 'string') {
    const parsed = Date.parse(value)
    return Number.isFinite(parsed) ? Math.floor(parsed / 1000) : null
  }
  return null
}

/** 按时长分档：0 / 不到 1 小时 / 不到 4 小时 / 不到 8 小时 / 更久。 */
function level(liveSecs: number): 0 | 1 | 2 | 3 | 4 {
  if (liveSecs <= 0) return 0
  if (liveSecs < 3600) return 1
  if (liveSecs < 14400) return 2
  if (liveSecs < 28800) return 3
  return 4
}

const LEVEL_CLASS = [
  styles.dayEmpty,
  styles.dayL1,
  styles.dayL2,
  styles.dayL3,
  styles.dayL4,
]

export default function Calendar({ calendar, selected, onSelect }: CalendarProps) {
  const days = calendar
    .map((row) => ({ at: daySeconds(row.day), live: row.live_secs, failures: row.failures }))
    .filter((day): day is { at: number; live: number; failures: number } => day.at !== null)

  if (days.length === 0) {
    return <p className={relay.muted}>还没有可展示的日历数据。</p>
  }

  const lead = (new Date(days[0].at * 1000).getDay() + 6) % 7
  const today = new Date()
  const todayKey = `${today.getFullYear()}-${today.getMonth()}-${today.getDate()}`

  return (
    <div className={styles.calendarWrap}>
      <div className={styles.calendarHead}>
        <span className={styles.calendarRange}>
          {new Date(days[0].at * 1000).toLocaleDateString('zh-CN')} —{' '}
          {new Date(days[days.length - 1].at * 1000).toLocaleDateString('zh-CN')}
        </span>
        <span className={styles.calendarLegend}>
          <i className={styles.dayL1} />
          <i className={styles.dayL2} />
          <i className={styles.dayL3} />
          <i className={styles.dayL4} />
          <span className={relay.muted}>越深时长越长</span>
        </span>
      </div>
      <div className={styles.calendarGrid}>
        {WEEKDAYS.map((name) => (
          <span key={name} className={styles.weekday}>
            {name}
          </span>
        ))}
        {Array.from({ length: lead }, (_, index) => (
          <span key={`lead-${index}`} />
        ))}
        {days.map((day) => {
          const date = new Date(day.at * 1000)
          const key = `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`
          const active = selected === day.at
          return (
            <button
              key={day.at}
              type="button"
              className={[styles.dayCell, LEVEL_CLASS[level(day.live)], active && styles.dayActive]
                .filter(Boolean)
                .join(' ')}
              aria-pressed={active}
              title={`${key}：推流 ${formatDurationSec(day.live)}，失败 ${day.failures} 次`}
              onClick={() => onSelect(active ? null : day.at)}
            >
              <span className={styles.dayNumber}>{date.getDate()}</span>
              {day.live > 0 ? <span className={styles.dayLive}>{formatDurationSec(day.live)}</span> : null}
              {key === todayKey ? <span className={styles.dayToday} aria-hidden="true" /> : null}
            </button>
          )
        })}
      </div>
    </div>
  )
}
