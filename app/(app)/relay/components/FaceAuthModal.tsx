'use client'

import { Banner, Button, Toast, Typography } from '@douyinfe/semi-ui'
import { QRCodeSVG } from 'qrcode.react'
import FormDialog from '@/app/ui/shell/FormDialog'
import styles from '@/app/ui/relay/relay.module.scss'
import pageStyles from './relay-overview.module.scss'

const { Text } = Typography

export interface FaceAuthModalProps {
  visible: boolean
  qrUrl: string | null
  message: string
  checking: boolean
  onCheck: () => void
  onClose: () => void
}

export default function FaceAuthModal({
  visible,
  qrUrl,
  message,
  checking,
  onCheck,
  onClose,
}: FaceAuthModalProps) {
  const copyLink = async () => {
    if (!qrUrl) return
    try {
      await navigator.clipboard.writeText(qrUrl)
      Toast.success('人脸验证链接已复制')
    } catch {
      Toast.error('复制失败，请手动复制下方链接')
    }
  }

  return (
    <FormDialog
      title="需要人脸验证"
      visible={visible}
      size="sm"
      okText="重新检查"
      onOk={onCheck}
      confirmLoading={checking}
      onCancel={onClose}
    >
      <div className={pageStyles.dialogBody}>
        <Banner
          className={styles.banner}
          type="warning"
          closeIcon={null}
          title={message || 'B 站要求完成人脸验证后才能开播'}
          description="请在手机 B 站客户端扫码并按提示完成验证，然后点「重新检查」再次开播。"
        />
        {qrUrl ? (
          <>
            <div className={pageStyles.qrBox}>
              <QRCodeSVG value={qrUrl} size={220} level="M" marginSize={2} />
            </div>
            <div className={pageStyles.linkRow}>
              <Text className={styles.monoHint} ellipsis={{ showTooltip: true }}>
                {qrUrl}
              </Text>
              <Button size="small" theme="light" onClick={() => void copyLink()}>
                复制链接
              </Button>
            </div>
          </>
        ) : (
          <Text type="tertiary" size="small">
            服务端没有返回验证链接。请在 B 站客户端完成验证后重新检查；仍失败时请查看实时日志。
          </Text>
        )}
        <Text type="tertiary" size="small">
          二维码由本地根据验证链接生成，不会上传到第三方。
        </Text>
      </div>
    </FormDialog>
  )
}
