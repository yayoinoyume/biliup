'use client'

import type { RelayEventKind } from '@/app/lib/relay-events'

export const RELAY_HEALTH_KEY = 'relay:health'
export const RELAY_VERSION_KEY = 'relay:version'
export const RELAY_STATUS_KEY = 'relay:status'
export const RELAY_NETWORK_KEY = 'relay:network-status'
export const RELAY_DEPS_KEY = 'relay:deps-status'
export const RELAY_SETUP_KEY = 'relay:setup-status'
export const RELAY_SETUP_LOGIN_KEY = 'relay:setup-login-status'
export const RELAY_CONFIG_KEY = 'relay:config'
export const RELAY_CHANNELS_KEY = 'relay:channels'
export const RELAY_HOLODEX_AUTH_KEY = 'relay:holodex-auth'
export const RELAY_STREAMS_CHANNELS_KEY = 'relay:streams:channels'
export const RELAY_STREAMS_FAVORITES_KEY = 'relay:streams:favorites'

export const RELAY_OVERVIEW_KEYS = [
  RELAY_HEALTH_KEY,
  RELAY_VERSION_KEY,
  RELAY_STATUS_KEY,
  RELAY_NETWORK_KEY,
  RELAY_DEPS_KEY,
  RELAY_SETUP_KEY,
  RELAY_CONFIG_KEY,
  RELAY_CHANNELS_KEY,
  RELAY_HOLODEX_AUTH_KEY,
  RELAY_STREAMS_CHANNELS_KEY,
  RELAY_STREAMS_FAVORITES_KEY,
] as const

export const RELAY_EVENT_KEYS: Record<RelayEventKind, readonly string[]> = {
  status: [RELAY_STATUS_KEY, RELAY_NETWORK_KEY],
  cluster: [RELAY_STATUS_KEY, RELAY_NETWORK_KEY],
  config: [RELAY_STATUS_KEY, RELAY_CONFIG_KEY, RELAY_CHANNELS_KEY, RELAY_DEPS_KEY, RELAY_SETUP_KEY],
  holodex: [
    RELAY_STREAMS_CHANNELS_KEY,
    RELAY_STREAMS_FAVORITES_KEY,
    RELAY_HOLODEX_AUTH_KEY,
    RELAY_STATUS_KEY,
  ],
  history: [],
  refresh: RELAY_OVERVIEW_KEYS,
}
