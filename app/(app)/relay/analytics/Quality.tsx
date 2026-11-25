'use client'

/**
 * 推流质量趋势：码率（kbps）与帧率画在一张图上，各自带 min / max 包络。
 * 与观众数图共用 `Plot`，时间轴与图例行为完全一致。
 */

import { useMemo } from 'react'
import type { RelayHistoryStats } from '@/app/lib/relay'
import Relay from '@/app/ui/relay/relay.module.scss'
import Plot, { type PlotSeries } from './Plot'

export interface QualityProps {
  stats: RelayHistoryStats
}

export default function Quality({ stats }: QualityProps) {
  const series = useMemo<PlotSeries[]>(() => {
    const lines: PlotSeries[] = []
    const bitrate = stats.series.bitrate
    if (bitrate) {
      lines.push({ label: '码率峰值', color: '--semi-green-3', alpha: 0.5, values: bitrate.max })
      lines.push({ label: '码率', color: '--semi-green-5', values: bitrate.mean })
      lines.push({ label: '码率谷值', color: '--semi-green-3', alpha: 0.5, values: bitrate.min })
    }
    const fps = stats.series.fps
    if (fps) {
      lines.push({ label: '帧率', color: '--semi-purple-5', values: fps.mean })
    }
    return lines
  }, [stats])

  if (series.length === 0 || !series.some((line) => line.values.some((value) => value !== null))) {
    return <p className={Relay.muted}>这段区间没有码率或帧率采样。</p>
  }

  return (
    <Plot
      label="码率与帧率趋势"
      ts={stats.ts}
      series={series}
      height={220}
      format={(value) => String(Math.round(value))}
    />
  )
}
