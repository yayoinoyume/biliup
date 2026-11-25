'use client'

/**
 * uPlot 的薄封装：创建 / 尺寸跟随 / 卸载销毁都在这里，页面组件只管给数据。
 *
 * - `ResizeObserver` 跟随容器宽度，卸载时 `destroy()` + `disconnect()`，不留 canvas 与观察者；
 * - 颜色一律现读 Semi 的 CSS 变量（画的时候才取），主题切换靠 `theme-mode` 属性观察触发重绘；
 * - `spanGaps: false`：后端用 `null` 表示断档，断档就该是空白而不是连一条假线。
 */

import { useEffect, useMemo, useRef } from 'react'
import uPlot from 'uplot'
import 'uplot/dist/uPlot.min.css'
import styles from './analytics.module.scss'

export interface PlotSeries {
  label: string
  /** Semi 的 RGB 三元组变量名，例如 `--semi-blue-5`。 */
  color: string
  /** 0..1；min/max 包络用半透明。 */
  alpha?: number
  values: (number | null)[]
}

export interface PlotProps {
  /** 图的语义名称，给读屏用。 */
  label: string
  ts: number[]
  series: PlotSeries[]
  height?: number
  format: (value: number) => string
}

const FONT = '11px system-ui, -apple-system, "Segoe UI", sans-serif'
const FALLBACK_RGB = '128,128,128'

function themeValue(name: string, fallback: string): string {
  if (typeof document === 'undefined') return fallback
  const value = getComputedStyle(document.body).getPropertyValue(name).trim()
  return value || fallback
}

const rgbOf = (name: string) => themeValue(name, FALLBACK_RGB)
const gridOf = () => themeValue('--semi-color-border', 'rgba(28,31,35,0.08)')
const axisOf = () => themeValue('--semi-color-text-2', 'rgba(28,31,35,0.62)')

export default function Plot({ label, ts, series, height = 240, format }: PlotProps) {
  const hostRef = useRef<HTMLDivElement>(null)
  const plotRef = useRef<uPlot | null>(null)
  const seriesRef = useRef(series)
  const tsRef = useRef(ts)
  const formatRef = useRef(format)

  // 建图 effect 只跟「哪几条线 + 高度」走，最新数据与格式化函数放 ref 里，避免每次采样都重建实例
  useEffect(() => {
    seriesRef.current = series
    tsRef.current = ts
    formatRef.current = format
  })

  const signature = series.map((line) => line.label).join('|')
  const data = useMemo<uPlot.AlignedData>(
    () => [ts, ...series.map((line) => line.values)],
    [ts, series],
  )

  useEffect(() => {
    const host = hostRef.current
    if (!host) return
    const lines = seriesRef.current
    const options: uPlot.Options = {
      width: Math.max(1, Math.floor(host.clientWidth)),
      height,
      // 图例用 uPlot 自带的表格：多序列时需要知道哪条线是什么颜色
      legend: { show: lines.length > 1, live: true },
      cursor: {
        y: false,
        drag: { x: false, y: false, setScale: false },
        points: { show: true, size: 6 },
      },
      select: { show: false, left: 0, top: 0, width: 0, height: 0 },
      scales: { x: { time: true } },
      series: [
        {},
        ...lines.map((line) => ({
          label: line.label,
          stroke: () => `rgba(${rgbOf(line.color)}, ${line.alpha ?? 1})`,
          width: line.alpha !== undefined && line.alpha < 1 ? 1 : 2,
          spanGaps: false,
          points: { show: false },
          value: (_u: uPlot, value: number | null) =>
            value === null || value === undefined ? '—' : formatRef.current(value),
        })),
      ],
      axes: [
        {
          stroke: axisOf,
          grid: { stroke: gridOf, width: 1 },
          ticks: { stroke: gridOf, width: 1, size: 4 },
          font: FONT,
          size: 24,
          gap: 4,
        },
        {
          stroke: axisOf,
          grid: { stroke: gridOf, width: 1 },
          ticks: { show: false },
          font: FONT,
          size: 56,
          gap: 6,
          values: (_u, splits) => splits.map((value) => formatRef.current(value)),
        },
      ],
    }
    const plot = new uPlot(options, [tsRef.current, ...lines.map((line) => line.values)], host)
    plotRef.current = plot
    const observer = new ResizeObserver(() => {
      const width = Math.floor(host.clientWidth)
      if (width > 0 && width !== plot.width) plot.setSize({ width, height })
    })
    observer.observe(host)
    // 主题切换只改 body 上的 theme-mode，不给事件；重绘时才重新读 CSS 变量
    const theme = new MutationObserver(() => plot.redraw())
    theme.observe(document.body, { attributes: true, attributeFilter: ['theme-mode'] })
    return () => {
      observer.disconnect()
      theme.disconnect()
      plot.destroy()
      plotRef.current = null
    }
  }, [signature, height])

  useEffect(() => {
    const plot = plotRef.current
    if (!plot) return
    plot.setData(data)
  }, [data])

  return (
    <div
      className={styles.plotHost}
      ref={hostRef}
      style={{ height }}
      role="img"
      aria-label={label}
    />
  )
}
