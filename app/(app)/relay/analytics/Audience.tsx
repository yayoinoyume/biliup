'use client'

/**
 * 观众数趋势：`/history/stats` 的 `online`（同时在线）与 `watched`（平台累计观看）。
 * min / max 是同一指标在每个采样窗口里的包络，用半透明细线画出来，
 * 中间那条实线是均值——断档处后端给 `null`，图上就是空白。
 */

import { useMemo } from 'react'
import type { RelayHistoryStats } from '@/app/lib/relay'
import Relay from '@/app/ui/relay/relay.module.scss'
import Plot, { type PlotSeries } from './Plot'

const people = (value: number) => String(Math.round(value))

export interface AudienceProps {
  stats: RelayHistoryStats
}

export default function Audience({ stats }: AudienceProps) {
  const series = useMemo<PlotSeries[]>(() => {
    const lines: PlotSeries[] = []
    const online = stats.series.online
    if (online) {
      lines.push({ label: '峰值', color: '--semi-blue-3', alpha: 0.5, values: online.max })
      lines.push({ label: '均值', color: '--semi-blue-5', values: online.mean })
      lines.push({ label: '谷值', color: '--semi-blue-3', alpha: 0.5, values: online.min })
    }
    const watched = stats.series.watched
    if (watched) {
      lines.push({ label: '平台已看', color: '--semi-orange-5', values: watched.mean })
    }
    return lines
  }, [stats])

  if (series.length === 0 || !series.some((line) => line.values.some((value) => value !== null))) {
    return <p className={Relay.muted}>这段区间没有观众数采样。</p>
  }

  return <Plot label="观众数趋势" ts={stats.ts} series={series} format={people} />
}
