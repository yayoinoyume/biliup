'use client'

/**
 * 覆盖信息：每个节点的推流时长、失败数与自报在线观测，以及各节点的历史起点。
 * `node_segments` 在契约里是 `unknown[]`，前端不猜形状，只报条数；
 * 没被覆盖到的节点单独列出来——它们的数据缺失会影响上面所有汇总。
 */

import type { RelayHistorySummary } from '@/app/lib/relay'
import { formatDurationSec, formatUnixSec } from '@/app/lib/relay-format'
import relay from '@/app/ui/relay/relay.module.scss'
import styles from './analytics.module.scss'

export interface NodesProps {
  summary: RelayHistorySummary
}

export default function Nodes({ summary }: NodesProps) {
  const nodes = summary.per_node
  const partial = summary.partial_nodes ?? []
  const missing = summary.missing_nodes
  const segments = summary.node_segments

  return (
    <div className={relay.stack}>
      <div className={relay.tableWrap}>
        <table className={styles.table}>
          <thead>
            <tr>
              <th scope="col">节点</th>
              <th scope="col">推流时长</th>
              <th scope="col">失败</th>
              <th scope="col">停机</th>
              <th scope="col">节点观测</th>
              <th scope="col">历史起点</th>
              <th scope="col">最后上报</th>
            </tr>
          </thead>
          <tbody>
            {nodes.map((node) => (
              <tr key={node.node_id}>
                <td className={relay.mono}>{node.node_id}</td>
                <td>{formatDurationSec(node.live_secs)}</td>
                <td>{node.failures}</td>
                <td>{formatDurationSec(node.down_secs)}</td>
                <td>{formatDurationSec(node.node_observed_secs)}</td>
                <td>{formatUnixSec(summary.coverage.per_node[node.node_id])}</td>
                <td>{formatUnixSec(node.last_seen_at)}</td>
              </tr>
            ))}
            {nodes.length === 0 ? (
              <tr>
                <td colSpan={7}>
                  <span className={relay.muted}>没有节点记录。</span>
                </td>
              </tr>
            ) : null}
          </tbody>
        </table>
      </div>

      <dl className={relay.kv}>
        <dt className={relay.kvKey}>覆盖起点</dt>
        <dd className={relay.kvValue}>{formatUnixSec(summary.coverage.from)}</dd>
        <dt className={relay.kvKey}>更早的保留边界</dt>
        <dd className={relay.kvValue}>
          {Object.keys(summary.coverage.partial_before).length === 0
            ? '无（全部节点覆盖整段区间）'
            : Object.entries(summary.coverage.partial_before)
                .map(([node, at]) => `${node}：${formatUnixSec(at)}`)
                .join('；')}
        </dd>
        <dt className={relay.kvKey}>分区段</dt>
        <dd className={relay.kvValue}>
          {segments.length} 段{summary.node_segments_more ? '（还有更多未展开）' : ''}
        </dd>
      </dl>

      {partial.length > 0 ? (
        <p className={relay.muted}>
          被节点自身的保留期截断：
          {partial.map((item) => `${item.node} 只保留 ${item.clamped_to_days} 天`).join('；')}
        </p>
      ) : null}
      {summary.clamped_to_days !== undefined ? (
        <p className={relay.muted}>
          查询区间被保留期截断到最近 {summary.clamped_to_days} 天
          {summary.clamp_scope === 'local' ? '（仅本节点）' : ''}。
        </p>
      ) : null}
      {missing.length > 0 ? (
        <ul className={styles.reasons}>
          {missing.map((node) => (
            <li key={node.node_id}>
              <span className={relay.mono}>{node.node_id}</span>
              <span className={relay.muted}>
                {node.reason}
                {node.detail ? ` · ${node.detail}` : ''}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
    </div>
  )
}
