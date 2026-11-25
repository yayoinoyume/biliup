'use client'

import { useRef, useState } from 'react'
import { Banner, Button, InputNumber, Switch, Toast, Typography } from '@douyinfe/semi-ui'
import { IconCamera, IconUpload } from '@douyinfe/semi-icons'
import useSWR from 'swr'
import { FormDialog } from '@/app/ui/shell'
import { useMe } from '@/app/lib/use-me'
import {
  relayCaptureCropFrame,
  relayGetCrop,
  relayRestartStream,
  relayUpdateCrop,
} from '@/app/lib/relay'
import type { RelayCropPlatform } from '@/app/lib/relay'
import { relayErrorMessage } from '@/app/ui/relay/RelayError'
import base from '@/app/ui/relay/relay.module.scss'
import styles from './manage.module.scss'

const { Text } = Typography

type Rect = { x: number; y: number; width: number; height: number }
type Size = { width: number; height: number }
type Notice = { type: 'danger' | 'warning' | 'success' | 'info'; text: string }
type Draft = { enabled: boolean; rect: Rect }

const EMPTY_RECT: Rect = { x: 0, y: 0, width: 0, height: 0 }

const PLATFORM_LABELS: Record<RelayCropPlatform, string> = {
  youtube: 'YouTube',
  twitch: 'Twitch',
  niconico: 'Niconico',
}

function clampInt(value: number, min: number, max: number): number {
  if (!Number.isFinite(value)) return min
  return Math.min(Math.max(Math.round(value), min), max)
}

function clampRect(rect: Rect, size: Size): Rect {
  const x = clampInt(rect.x, 0, Math.max(0, size.width - 1))
  const y = clampInt(rect.y, 0, Math.max(0, size.height - 1))
  return {
    x,
    y,
    width: clampInt(rect.width, 1, Math.max(1, size.width - x)),
    height: clampInt(rect.height, 1, Math.max(1, size.height - y)),
  }
}

function rectBetween(start: { x: number; y: number }, end: { x: number; y: number }, size: Size): Rect {
  const left = Math.min(start.x, end.x)
  const top = Math.min(start.y, end.y)
  const right = Math.min(Math.max(start.x, end.x) + 1, size.width)
  const bottom = Math.min(Math.max(start.y, end.y) + 1, size.height)
  return { x: left, y: top, width: Math.max(1, right - left), height: Math.max(1, bottom - top) }
}

function loadSize(url: string): Promise<Size> {
  return new Promise((resolve, reject) => {
    const image = new Image()
    image.onload = () => resolve({ width: image.naturalWidth, height: image.naturalHeight })
    image.onerror = () => reject(new Error('图片无法解码，请换一张 JPG 或 PNG'))
    image.src = url
  })
}

export default function CropModal({
  platform,
  visible,
  onClose,
}: {
  platform: RelayCropPlatform
  visible: boolean
  onClose: () => void
}) {
  const { can } = useMe()
  const canCapture = can('relay.control')
  const canEdit = can('relay.config')
  const crop = useSWR(['relay:crop', platform], () => relayGetCrop(platform), {
    revalidateOnFocus: false,
  })
  const [draft, setDraft] = useState<Draft | null>(null)
  const [selection, setSelection] = useState<Rect | null>(null)
  const [frameUrl, setFrameUrl] = useState<string | null>(null)
  const [frameSize, setFrameSize] = useState<Size | null>(null)
  const [capturing, setCapturing] = useState(false)
  const [restarting, setRestarting] = useState(false)
  const [saved, setSaved] = useState(false)
  const [notice, setNotice] = useState<Notice | null>(null)
  const dragRef = useRef<{ x: number; y: number } | null>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const serverRect = crop.data ?? EMPTY_RECT
  const serverEnabled = crop.data !== null && crop.data !== undefined
  const enabled = draft?.enabled ?? serverEnabled
  const rect = draft?.rect ?? serverRect

  const updateRect = (next: Rect | ((prev: Rect) => Rect)) => {
    setDraft((prev) => ({
      enabled: prev?.enabled ?? serverEnabled,
      rect: typeof next === 'function' ? next(prev?.rect ?? serverRect) : next,
    }))
  }

  const adoptFrame = async (url: string, message: string) => {
    try {
      const size = await loadSize(url)
      if (size.width < 2 || size.height < 2) throw new Error('画面尺寸过小，无法作为裁剪基准')
      setFrameUrl(url)
      setFrameSize(size)
      updateRect((prev) =>
        prev.width > 1 && prev.height > 1
          ? clampRect(prev, size)
          : { x: 0, y: 0, width: size.width, height: size.height },
      )
      setNotice({ type: 'success', text: `${message}（原始尺寸 ${size.width} × ${size.height}）` })
    } catch (failure) {
      setNotice({ type: 'danger', text: relayErrorMessage(failure) })
    }
  }

  const capture = async () => {
    setCapturing(true)
    setNotice(null)
    try {
      const url = await relayCaptureCropFrame(platform)
      await adoptFrame(url, '已抓到当前画面')
    } catch (failure) {
      setNotice({ type: 'danger', text: relayErrorMessage(failure) })
    } finally {
      setCapturing(false)
    }
  }

  const pickFile = (event: React.ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0]
    event.target.value = ''
    if (!file) return
    const reader = new FileReader()
    reader.onload = () => {
      if (typeof reader.result !== 'string') {
        setNotice({ type: 'danger', text: '读取图片失败' })
        return
      }
      void adoptFrame(reader.result, `已载入本地静帧 ${file.name}`)
    }
    reader.onerror = () => setNotice({ type: 'danger', text: '读取图片失败' })
    reader.readAsDataURL(file)
  }

  const pointOf = (event: React.PointerEvent<HTMLDivElement>): { x: number; y: number } | null => {
    if (!frameSize) return null
    const box = event.currentTarget.getBoundingClientRect()
    if (box.width <= 0 || box.height <= 0) return null
    return {
      x: clampInt(((event.clientX - box.left) / box.width) * frameSize.width, 0, Math.max(0, frameSize.width - 1)),
      y: clampInt(((event.clientY - box.top) / box.height) * frameSize.height, 0, Math.max(0, frameSize.height - 1)),
    }
  }

  const startDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    if (!frameSize || event.button !== 0) return
    const point = pointOf(event)
    if (!point) return
    event.currentTarget.setPointerCapture(event.pointerId)
    dragRef.current = point
    setSelection({ x: point.x, y: point.y, width: 1, height: 1 })
  }

  const moveDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const start = dragRef.current
    if (!start || !frameSize) return
    const point = pointOf(event)
    if (point) setSelection(rectBetween(start, point, frameSize))
  }

  const endDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    const start = dragRef.current
    dragRef.current = null
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
    if (!start || !frameSize) {
      setSelection(null)
      return
    }
    const point = pointOf(event)
    if (point) updateRect(rectBetween(start, point, frameSize))
    setSelection(null)
  }

  const setField = (key: keyof Rect, value: number | string) => {
    const next = typeof value === 'number' ? value : Number(value)
    if (!Number.isFinite(next)) return
    updateRect((prev) => ({ ...prev, [key]: Math.round(next) }))
  }

  const save = async () => {
    setNotice(null)
    try {
      if (!enabled) {
        const message = await relayUpdateCrop({ platform, enabled: false })
        Toast.success(message ?? `${PLATFORM_LABELS[platform]} 的裁剪已关闭`)
        setSaved(true)
        void crop.mutate()
        return
      }
      const seed =
        rect.width >= 1 && rect.height >= 1
          ? rect
          : frameSize
            ? { x: 0, y: 0, width: frameSize.width, height: frameSize.height }
            : rect
      if (seed.width < 1 || seed.height < 1) {
        setNotice({
          type: 'danger',
          text: frameSize ? '裁剪区域的宽度与高度必须大于 0' : '先抓帧或填好宽高，再保存裁剪',
        })
        return
      }
      const next = frameSize
        ? clampRect(seed, frameSize)
        : {
            x: Math.max(0, Math.round(seed.x)),
            y: Math.max(0, Math.round(seed.y)),
            width: Math.max(1, Math.round(seed.width)),
            height: Math.max(1, Math.round(seed.height)),
          }
      updateRect(next)
      const message = await relayUpdateCrop({
        platform,
        enabled: true,
        x: next.x,
        y: next.y,
        width: next.width,
        height: next.height,
      })
      Toast.success(message ?? `${PLATFORM_LABELS[platform]} 的裁剪已保存`)
      setSaved(true)
      void crop.mutate()
    } catch (failure) {
      setNotice({ type: 'danger', text: relayErrorMessage(failure) })
    }
  }

  const restart = async () => {
    setRestarting(true)
    try {
      const message = await relayRestartStream()
      Toast.success(message ?? '已重启转播')
      setSaved(false)
    } catch (failure) {
      setNotice({ type: 'danger', text: relayErrorMessage(failure) })
    } finally {
      setRestarting(false)
    }
  }

  const active = selection ?? rect
  const percent = (value: number, total: number) => `${(value / Math.max(1, total)) * 100}%`

  return (
    <FormDialog
      title={`${PLATFORM_LABELS[platform]} 画面裁剪`}
      size="lg"
      visible={visible}
      okText={canEdit ? '保存裁剪' : undefined}
      okDisabled={!canEdit}
      onOk={save}
      onCancel={onClose}
    >
      <div className={base.stack}>
        {notice ? <Banner type={notice.type} closeIcon={null} description={notice.text} /> : null}
        {crop.error ? (
          <Banner
            type="danger"
            closeIcon={null}
            title="读取裁剪配置失败"
            description={relayErrorMessage(crop.error)}
          />
        ) : null}

        <div className={base.toolbar}>
          {platform === 'niconico' ? (
            <Text type="tertiary" size="small">
              Niconico 无法从 HLS 抓帧，请选一张本地静帧，或直接按已知像素填坐标。
            </Text>
          ) : (
            <Button
              icon={<IconCamera />}
              loading={capturing}
              disabled={!canCapture}
              onClick={() => {
                void capture()
              }}
            >
              抓取当前画面
            </Button>
          )}
          <Button icon={<IconUpload />} onClick={() => fileRef.current?.click()}>
            选择本地静帧
          </Button>
          <input ref={fileRef} type="file" accept="image/*" hidden onChange={pickFile} />
          {platform !== 'niconico' && !canCapture ? (
            <Text type="tertiary" size="small">
              实时抓帧需要「启停转播」权限。
            </Text>
          ) : null}
        </div>

        {frameUrl && frameSize ? (
          <div
            className={styles.preview}
            style={{ aspectRatio: `${frameSize.width} / ${frameSize.height}` }}
            onPointerDown={startDrag}
            onPointerMove={moveDrag}
            onPointerUp={endDrag}
            onPointerCancel={endDrag}
            role="img"
            aria-label={`${PLATFORM_LABELS[platform]} 画面预览，可拖拽框选裁剪区域`}
          >
            <div className={styles.pane} style={{ backgroundImage: `url(${frameUrl})` }} />
            <div
              className={styles.selection}
              style={{
                left: percent(active.x, frameSize.width),
                top: percent(active.y, frameSize.height),
                width: percent(active.width, frameSize.width),
                height: percent(active.height, frameSize.height),
              }}
            />
          </div>
        ) : (
          <div className={styles.emptyPreview}>
            <Text type="tertiary" size="small">
              {platform === 'niconico'
                ? '还没有画面：选一张本地静帧后即可框选。'
                : canCapture
                  ? '点「抓取当前画面」拿到直播截图后框选，或直接填写已知的像素坐标。'
                  : '暂无画面：没有抓帧权限时可以直接填写已知的像素坐标。'}
            </Text>
          </div>
        )}

        <div className={base.toolbar}>
          <Switch
            checked={enabled}
            onChange={(next) => setDraft({ enabled: next, rect })}
            disabled={!canEdit}
            aria-label="启用画面裁剪"
          />
          <Text size="small">启用裁剪</Text>
          {frameSize ? (
            <Text type="tertiary" size="small">
              原始画面 {frameSize.width} × {frameSize.height}
            </Text>
          ) : null}
        </div>

        <div className={styles.fields}>
          <div className={base.field}>
            <span className={base.fieldLabel}>左边距 x</span>
            <InputNumber
              value={rect.x}
              min={0}
              max={frameSize ? Math.max(0, frameSize.width - 1) : undefined}
              step={1}
              precision={0}
              disabled={!canEdit || !enabled}
              onChange={(next) => setField('x', next)}
              style={{ width: '100%' }}
            />
          </div>
          <div className={base.field}>
            <span className={base.fieldLabel}>上边距 y</span>
            <InputNumber
              value={rect.y}
              min={0}
              max={frameSize ? Math.max(0, frameSize.height - 1) : undefined}
              step={1}
              precision={0}
              disabled={!canEdit || !enabled}
              onChange={(next) => setField('y', next)}
              style={{ width: '100%' }}
            />
          </div>
          <div className={base.field}>
            <span className={base.fieldLabel}>宽度</span>
            <InputNumber
              value={rect.width}
              min={0}
              max={frameSize ? frameSize.width : undefined}
              step={1}
              precision={0}
              disabled={!canEdit || !enabled}
              onChange={(next) => setField('width', next)}
              style={{ width: '100%' }}
            />
          </div>
          <div className={base.field}>
            <span className={base.fieldLabel}>高度</span>
            <InputNumber
              value={rect.height}
              min={0}
              max={frameSize ? frameSize.height : undefined}
              step={1}
              precision={0}
              disabled={!canEdit || !enabled}
              onChange={(next) => setField('height', next)}
              style={{ width: '100%' }}
            />
          </div>
        </div>

        {saved ? (
          <div className={base.toolbar}>
            <Text type="tertiary" size="small">
              已保存。重启转播后新画面参数才会应用。
            </Text>
            {can('relay.control') ? (
              <Button
                loading={restarting}
                onClick={() => {
                  void restart()
                }}
              >
                重启转播立即生效
              </Button>
            ) : null}
          </div>
        ) : (
          <Text type="tertiary" size="small">
            坐标按抓帧原始像素填写：x/y 是左上角，宽高必须落在画面内；保存后重启转播生效。
          </Text>
        )}
      </div>
    </FormDialog>
  )
}
