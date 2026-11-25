'use client'

/**
 * 敏感密钥与代理输入框。
 *
 * 后端读模型只回掩码（同长度的一串点）与 `*_configured` 布尔，写模型要求：
 * 已配置且用户没输入新值时回传空口令位并声明 `keep_*`，否则整条请求被当成 EDIT_INVALID。
 * 这个组件把那一套收敛成一个普通受控输入：留空 = 保留，勾选“清除”才真的抹掉。
 */

import { Checkbox, Input, Tag, TextArea, Typography } from '@douyinfe/semi-ui'
import styles from './relay.module.scss'

const { Text } = Typography

export interface MaskedSecretInputProps {
  id?: string
  label: string
  /** 用户当前输入的新值；留空表示沿用已保存的值。 */
  value: string
  onChange: (next: string) => void
  /** 后端是否已保存过这项凭据。 */
  configured: boolean
  /** 后端回的掩码，用于占位提示。 */
  mask?: string | null
  placeholder?: string
  help?: string
  disabled?: boolean
  /** 多行输入（Netscape 格式 Cookie 之类）。 */
  multiline?: boolean
  rows?: number
  /** 是否显示“清除已保存”的勾选框；不给回调就不显示。 */
  clearRequested?: boolean
  onClearChange?: (next: boolean) => void
  autoComplete?: string
}

export default function MaskedSecretInput({
  id,
  label,
  value,
  onChange,
  configured,
  mask,
  placeholder,
  help,
  disabled = false,
  multiline = false,
  rows = 6,
  clearRequested = false,
  onClearChange,
  autoComplete = 'off',
}: MaskedSecretInputProps) {
  const placeholderText =
    placeholder ??
    (configured
      ? '已保存，留空表示不修改'
      : '尚未配置，填写后保存')

  const statusTag = configured ? (
    <Tag size="small" color="green" type="light">
      已配置
    </Tag>
  ) : (
    <Tag size="small" type="light">
      未配置
    </Tag>
  )

  return (
    <div className={styles.field}>
      <div className={styles.fieldHead}>
        <label className={styles.fieldLabel} htmlFor={id}>
          {label}
        </label>
        {statusTag}
      </div>
      {multiline ? (
        <TextArea
          id={id}
          value={value}
          onChange={onChange}
          placeholder={placeholderText}
          disabled={disabled || clearRequested}
          rows={rows}
          autosize={false}
          spellCheck={false}
        />
      ) : (
        <Input
          id={id}
          type="password"
          value={value}
          onChange={onChange}
          placeholder={placeholderText}
          disabled={disabled || clearRequested}
          autoComplete={autoComplete}
          spellCheck={false}
        />
      )}
      {configured && mask ? (
        <Text type="tertiary" size="small" className={styles.monoHint}>
          当前：{mask}
        </Text>
      ) : null}
      {help ? (
        <Text type="tertiary" size="small">
          {help}
        </Text>
      ) : null}
      {onClearChange && configured ? (
        <Checkbox
          checked={clearRequested}
          onChange={(event) => onClearChange(event.target.checked === true)}
          disabled={disabled}
        >
          保存时清除已保存的内容
        </Checkbox>
      ) : null}
    </div>
  )
}
