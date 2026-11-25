'use client'

import { isRelayApiError } from '@/app/lib/relay'

let tail: Promise<unknown> = Promise.resolve()

export function enqueueRelaySave<T>(task: () => Promise<T>): Promise<T> {
  const run = tail.then(task, task)
  tail = run.then(
    () => undefined,
    () => undefined,
  )
  return run
}

export function isConflictError(error: unknown): boolean {
  return isRelayApiError(error) && error.status === 409
}
