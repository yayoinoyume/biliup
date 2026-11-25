'use client'

import { useMemo, useRef, useState } from 'react'
import { Button, Form, Input, Table, Tag, Toast, Typography } from '@douyinfe/semi-ui'
import { IconDelete, IconEdit2Stroked, IconPlus, IconSearch } from '@douyinfe/semi-icons'
import type { FormApi } from '@douyinfe/semi-ui/lib/es/form'
import type { ColumnProps } from '@douyinfe/semi-ui/lib/es/table'
import useSWR from 'swr'
import { FormDialog } from '@/app/ui/shell'
import { useMe } from '@/app/lib/use-me'
import {
  relayAddArea,
  relayDeleteArea,
  relayManageAreas,
  relayUpdateAreaManage,
} from '@/app/lib/relay'
import type { RelayAddAreaRequest, RelayArea } from '@/app/lib/relay'
import AreaCatalogPicker, {
  RELAY_AREA_GROUPS_KEY,
  loadRelayAreaGroups,
} from '@/app/ui/relay/AreaCatalogPicker'
import type { RelayAreaGroup } from '@/app/ui/relay/AreaCatalogPicker'
import RelayError, { RelayEmptyState, relayErrorMessage } from '@/app/ui/relay/RelayError'
import ConfirmDialog from './ConfirmDialog'
import { joinList, splitList } from './form-values'
import styles from '@/app/ui/relay/relay.module.scss'

const { Text, Title } = Typography

const AREAS_KEY = 'relay:manage-areas'

type AreaValues = {
  id: number | string
  name: string
  title_keywords: string
  aliases: string
}

function AreaEditor({
  area,
  existingIds,
  onClose,
  onSaved,
}: {
  area: RelayArea | null
  existingIds: number[]
  onClose: () => void
  onSaved: () => void
}) {
  const formApi = useRef<FormApi<AreaValues>>(undefined)
  const { data: groups } = useSWR<RelayAreaGroup[]>(RELAY_AREA_GROUPS_KEY, loadRelayAreaGroups, {
    revalidateOnFocus: false,
  })
  const [picked, setPicked] = useState<number | null>(null)

  const applyCatalog = (areaId: number) => {
    setPicked(areaId)
    formApi.current?.setValue('id', areaId)
    const match = (groups ?? [])
      .flatMap((group) => group.areas)
      .find((item) => item.id === areaId)
    if (match) formApi.current?.setValue('name', match.name)
  }

  const submit = async () => {
    let values: AreaValues
    try {
      values = (await formApi.current?.validate()) as AreaValues
    } catch {
      return
    }
    const id = Number(values.id)
    if (!Number.isInteger(id) || id <= 0) {
      formApi.current?.setError('id', '分区 ID 必须是正整数')
      return
    }
    if (!area && existingIds.includes(id)) {
      formApi.current?.setError('id', '这个分区 ID 已经在名单里')
      return
    }
    const payload: RelayAddAreaRequest = {
      id,
      name: values.name.trim(),
      title_keywords: splitList(values.title_keywords),
      aliases: splitList(values.aliases),
    }
    try {
      const message = area ? await relayUpdateAreaManage(payload) : await relayAddArea(payload)
      Toast.success(message ?? (area ? '分区已更新' : '分区已新增'))
      onSaved()
    } catch (failure) {
      Toast.error(relayErrorMessage(failure))
    }
  }

  return (
    <FormDialog
      title={area ? `编辑分区：${area.name}` : '新增分区'}
      size="md"
      visible
      okText={area ? '保存' : '新增'}
      onOk={submit}
      onCancel={onClose}
    >
      <Form<AreaValues>
        getFormApi={(api) => {
          formApi.current = api
        }}
        initValues={{
          id: area?.id ?? '',
          name: area?.name ?? '',
          title_keywords: joinList(area?.title_keywords),
          aliases: joinList(area?.aliases),
        }}
        labelPosition="top"
      >
        <Form.InputNumber
          field="id"
          label="分区 ID"
          disabled={Boolean(area)}
          min={1}
          precision={0}
          style={{ width: '100%' }}
          rules={[
            { required: true, message: '请填写分区 ID' },
            {
              validator: (_rule: unknown, value: number | string) => {
                const id = Number(value)
                return Number.isInteger(id) && id > 0
              },
              message: '分区 ID 必须是正整数',
            },
          ]}
        />
        <Form.Input
          field="name"
          label="分区名称"
          rules={[{ required: true, message: '请填写分区名称' }]}
        />
        <Form.Input
          field="title_keywords"
          label="标题关键词（逗号分隔）"
          placeholder="直播标题里出现这些词就归到这个分区"
        />
        <Form.Input field="aliases" label="别名（逗号分隔）" placeholder="分区常见的其他叫法" />
      </Form>

      {area ? null : (
        <div className={styles.field}>
          <div className={styles.fieldHead}>
            <span className={styles.fieldLabel}>从官方分区目录快速导入</span>
          </div>
          <AreaCatalogPicker value={picked} onChange={applyCatalog} />
        </div>
      )}

      <Text type="tertiary" size="small">
        选区 ID 与名称后仍可改；保存后立即写入本机 areas.json。
      </Text>
    </FormDialog>
  )
}

export default function AreasPanel() {
  const { can } = useMe()
  const canEdit = can('relay.config')
  const { data, error, isLoading, mutate } = useSWR(AREAS_KEY, relayManageAreas, {
    revalidateOnFocus: false,
  })
  const [query, setQuery] = useState('')
  const [editor, setEditor] = useState<{ mode: 'create' } | { mode: 'edit'; area: RelayArea } | null>(null)
  const [deleting, setDeleting] = useState<RelayArea | null>(null)

  const rows = useMemo(() => {
    const all = data?.areas ?? []
    const keyword = query.trim().toLowerCase()
    if (!keyword) return all
    return all.filter((area) => {
      const haystack = [String(area.id), area.name, ...area.title_keywords, ...area.aliases]
        .join(' ')
        .toLowerCase()
      return haystack.includes(keyword)
    })
  }, [data, query])

  const columns: ColumnProps<RelayArea>[] = [
    { title: 'ID', dataIndex: 'id', width: 90 },
    {
      title: '分区名称',
      dataIndex: 'name',
      width: 180,
      render: (_value, area) => <Text strong>{area.name}</Text>,
    },
    {
      title: '标题关键词',
      width: 280,
      render: (_value, area) =>
        area.title_keywords.length > 0 ? (
          <div className={styles.toolbar}>
            {area.title_keywords.slice(0, 4).map((keyword) => (
              <Tag key={keyword} size="small">
                {keyword}
              </Tag>
            ))}
            {area.title_keywords.length > 4 ? (
              <Text type="tertiary" size="small">
                还有 {area.title_keywords.length - 4} 个
              </Text>
            ) : null}
          </div>
        ) : (
          <Text type="tertiary" size="small">
            未设置
          </Text>
        ),
    },
    {
      title: '别名',
      width: 200,
      render: (_value, area) =>
        area.aliases.length > 0 ? (
          <Text size="small">{area.aliases.join('，')}</Text>
        ) : (
          <Text type="tertiary" size="small">
            —
          </Text>
        ),
    },
  ]

  if (canEdit) {
    columns.push({
      title: '操作',
      width: 110,
      render: (_value, area) => (
        <div className={styles.toolbar}>
          <Button
            size="small"
            theme="borderless"
            icon={<IconEdit2Stroked />}
            aria-label={`编辑 ${area.name}`}
            onClick={() => setEditor({ mode: 'edit', area })}
          />
          <Button
            size="small"
            type="danger"
            theme="borderless"
            icon={<IconDelete />}
            aria-label={`删除 ${area.name}`}
            onClick={() => setDeleting(area)}
          />
        </div>
      ),
    })
  }

  return (
    <div className={styles.stack}>
      {error ? (
        <RelayError
          error={error}
          title="分区规则加载失败"
          onRetry={() => {
            void mutate()
          }}
        />
      ) : null}

      <section className={styles.card}>
        <div className={styles.sectionTitle}>
          <div className={styles.stack}>
            <Title heading={6} style={{ margin: 0 }}>
              分区规则
            </Title>
            <Text type="tertiary" size="small">
              标题里命中关键词就自动落到对应分区；也可以直接引用官方分区目录。
            </Text>
          </div>
          <div className={styles.toolbar}>
            <Input
              value={query}
              onChange={setQuery}
              prefix={<IconSearch />}
              placeholder="搜索分区 ID、名称、关键词"
              showClear
              style={{ width: 240 }}
            />
            {canEdit ? (
              <Button icon={<IconPlus />} onClick={() => setEditor({ mode: 'create' })}>
                新增分区
              </Button>
            ) : null}
          </div>
        </div>

        <div className={styles.tableWrap}>
          <Table<RelayArea>
            columns={columns}
            dataSource={rows}
            rowKey="id"
            pagination={false}
            size="middle"
            loading={isLoading}
            scroll={{ x: 880 }}
            empty={
              <RelayEmptyState
                title={query.trim() ? '没有匹配的分区' : '还没有分区规则'}
                description={
                  query.trim()
                    ? '换个关键词，或者清空搜索框看全部分区。'
                    : canEdit
                      ? '点右上角「新增分区」，从官方目录导入或手填 ID。'
                      : '当前账号只能查看，分区由有「修改转播配置」权限的账号维护。'
                }
              />
            }
          />
        </div>
      </section>

      {editor ? (
        <AreaEditor
          key={editor.mode === 'edit' ? `edit-${editor.area.id}` : 'create'}
          area={editor.mode === 'edit' ? editor.area : null}
          existingIds={(data?.areas ?? []).map((area) => area.id)}
          onClose={() => setEditor(null)}
          onSaved={() => {
            setEditor(null)
            void mutate()
          }}
        />
      ) : null}

      <ConfirmDialog
        visible={Boolean(deleting)}
        title="删除分区"
        description={
          deleting
            ? `将删除分区「${deleting.name}」（ID ${deleting.id}）及其关键词与别名；已按这个分区投过的稿件不会变。`
            : ''
        }
        onConfirm={async () => {
          if (!deleting) return
          try {
            const message = await relayDeleteArea(deleting.id)
            Toast.success(message ?? '分区已删除')
            setDeleting(null)
            void mutate()
          } catch (failure) {
            Toast.error(relayErrorMessage(failure))
          }
        }}
        onCancel={() => setDeleting(null)}
      />
    </div>
  )
}
