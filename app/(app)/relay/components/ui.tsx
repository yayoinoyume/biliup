'use client'

import { Toast } from '@douyinfe/semi-ui'
import { useEffect, useRef, useState, type ReactNode } from 'react'
import { relayErrorMessage } from '@/app/ui/relay/RelayError'
import styles from '@/app/ui/relay/relay.module.scss'

export interface RelayTask {
  busy: boolean
  run: (task: () => Promise<unknown>, successText: string) => Promise<boolean>
}

export function useRelayTask(): RelayTask {
  const [busy, setBusy] = useState(false)
  const active = useRef(false)
  const run = async (task: () => Promise<unknown>, successText: string): Promise<boolean> => {
    if (active.current) return false
    active.current = true
    setBusy(true)
    try {
      const message = await task()
      Toast.success(typeof message === 'string' && message.trim() ? message.trim() : successText)
      return true
    } catch (error) {
      Toast.error(relayErrorMessage(error))
      return false
    } finally {
      active.current = false
      setBusy(false)
    }
  }
  return { busy, run }
}

export function useNow(active: boolean): number | null {
  const [now, setNow] = useState<number | null>(null)
  useEffect(() => {
    if (!active) return
    const timer = window.setInterval(() => {
      setNow(Date.now())
    }, 1000)
    return () => {
      window.clearInterval(timer)
    }
  }, [active])
  return now
}

export function StatusDot({ online }: { online: boolean }) {
  return <span className={styles.statusDot + ' ' + (online ? styles.online : styles.offline)} />
}

export function Field({
  label,
  hint,
  children,
}: {
  label: ReactNode
  hint?: ReactNode
  children: ReactNode
}) {
  return (
    <div className={styles.field}>
      <span className={styles.fieldLabel}>{label}</span>
      {children}
      {hint ? <span className={styles.muted}>{hint}</span> : null}
    </div>
  )
}
