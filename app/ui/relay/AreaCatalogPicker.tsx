'use client'

/**
 * B 站分区二级联动选择器：一级是官方父类（网游 / 手游 / 单机 …），二级是具体分区。
 *
 * 数据优先取 `/v1/relay/areas/catalog`（B 站官方两级表，带 `parent_name`）；
 * 官方表不可用时退回 `/v1/relay/areas`（本机 `areas.json`，没有父类，归到一组）。
 * P3-02 与 P3-03 共用同一个组件，别再各写一套。
 */

import { Select, Typography } from '@douyinfe/semi-ui'
import { useState } from 'react'
import useSWR from 'swr'
import { relayAreaCatalog, relayAreas } from '@/app/lib/relay'
import type { RelaySetupArea } from '@/app/lib/relay'
import styles from './relay.module.scss'

const { Text } = Typography

/** SWR 键：与组件解耦，页面可以自己 `mutate` 或预取。 */
export const RELAY_AREA_GROUPS_KEY = 'relay:area-groups'

const LOCAL_GROUP = '本机分区'

export interface RelayAreaGroup {
  parent: string
  areas: RelaySetupArea[]
}

function groupAreas(areas: RelaySetupArea[], fallbackParent: string): RelayAreaGroup[] {
  const groups = new Map<string, RelaySetupArea[]>()
  for (const area of areas) {
    const parent = area.parent_name?.trim() || fallbackParent
    const bucket = groups.get(parent)
    if (bucket) bucket.push(area)
    else groups.set(parent, [area])
  }
  return Array.from(groups, ([parent, list]) => ({
    parent,
    areas: [...list].sort((left, right) => left.id - right.id),
  })).sort((left, right) => left.parent.localeCompare(right.parent, 'zh-Hans-CN'))
}

/**
 * 拉分区选项：官方目录优先，失败后退回本机文档。
 * 两路都失败才抛错，让页面显示“分区列表暂不可用 + 重试”。
 */
export async function loadRelayAreaGroups(): Promise<RelayAreaGroup[]> {
  try {
    const catalog = await relayAreaCatalog()
    if (catalog.length > 0) return groupAreas(catalog, LOCAL_GROUP)
  } catch {
    // 官方目录要联网取；离线时退回本机分区，别把页面卡死
  }
  const document = await relayAreas()
  const local = (document.areas ?? []).map((area) => ({
    id: area.id,
    name: area.name,
    parent_name: LOCAL_GROUP,
  }))
  return groupAreas(local, LOCAL_GROUP)
}

export interface AreaCatalogPickerProps {
  /** 当前选中的分区 id；`null`/`undefined` 表示还没选。 */
  value?: number | null
  onChange: (areaId: number) => void
  disabled?: boolean
  /** 一级下拉的标签，默认「分区大类」。 */
  parentLabel?: string
  /** 二级下拉的标签，默认「具体分区」。 */
  areaLabel?: string
}

export default function AreaCatalogPicker({
  value,
  onChange,
  disabled = false,
  parentLabel = '分区大类',
  areaLabel = '具体分区',
}: AreaCatalogPickerProps) {
  const { data, error, isLoading, mutate } = useSWR<RelayAreaGroup[]>(
    RELAY_AREA_GROUPS_KEY,
    loadRelayAreaGroups,
    { revalidateOnFocus: false },
  )
  const [pickedParent, setPickedParent] = useState<string | null>(null)

  const groups = data ?? []
  const valueParent =
    value === undefined || value === null
      ? null
      : groups.find((group) => group.areas.some((area) => area.id === value))?.parent ?? null
  const activeParent =
    pickedParent && groups.some((group) => group.parent === pickedParent)
      ? pickedParent
      : valueParent ?? groups[0]?.parent ?? null
  const visibleAreas = groups.find((group) => group.parent === activeParent)?.areas ?? []
  const areaValue = activeParent === valueParent && value !== null && value !== undefined ? value : undefined
  const selectedName = value === undefined || value === null
    ? null
    : groups.flatMap((group) => group.areas).find((area) => area.id === value)?.name ?? null

  return (
    <div className={styles.areaPicker}>
      <div className={styles.areaPickerRow}>
        <Select
          prefix={parentLabel}
          value={activeParent ?? undefined}
          onChange={(next) => setPickedParent(String(next))}
          optionList={groups.map((group) => ({
            label: `${group.parent}（${group.areas.length}）`,
            value: group.parent,
          }))}
          disabled={disabled || isLoading || groups.length === 0}
          loading={isLoading}
          style={{ width: '100%' }}
          placeholder="选择分区大类"
        />
        <Select
          prefix={areaLabel}
          value={areaValue}
          onChange={(next) => onChange(Number(next))}
          optionList={visibleAreas.map((area) => ({
            label: `${area.name}（${area.id}）`,
            value: area.id,
          }))}
          disabled={disabled || isLoading || visibleAreas.length === 0}
          showClear={false}
          style={{ width: '100%' }}
          placeholder={visibleAreas.length === 0 ? '这类下没有分区' : '选择具体分区'}
        />
      </div>
      {error ? (
        <div className={styles.areaPickerError}>
          <Text type="danger" size="small">
            分区列表加载失败，可能没有联网取到 B 站官方分区。
          </Text>
          <Text
            type="tertiary"
            size="small"
            link
            onClick={() => {
              void mutate()
            }}
          >
            重试
          </Text>
        </div>
      ) : (
        <Text type="tertiary" size="small">
          {selectedName
            ? `当前分区：${selectedName}（${value}）`
            : '分区决定稿件落点；B 站官方目录不可用时会退回本机已保存的分区。'}
        </Text>
      )}
    </div>
  )
}
