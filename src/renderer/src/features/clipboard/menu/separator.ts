import type { ContextMenuItem } from '@renderer/composables/useContextMenu'

export const separator = (): ContextMenuItem => ({
  label: '---',
  action: () => undefined,
  disabled: true
})
