import type { Component } from 'vue'
import type { ClipboardPreview } from '../types'
import FilesPreview from './FilesPreview.vue'
import ImagePreview from './ImagePreview.vue'
import RichPreview from './RichPreview.vue'
import TablePreview from './TablePreview.vue'
import LinkPreview from './LinkPreview.vue'
import ContactPreview from './ContactPreview.vue'
import ColorPreview from './ColorPreview.vue'
import TextPreview from './TextPreview.vue'
import EmptyPreview from './EmptyPreview.vue'

export const previewComponents: Record<ClipboardPreview['kind'], Component> = {
  files: FilesPreview,
  image: ImagePreview,
  rich: RichPreview,
  table: TablePreview,
  url: LinkPreview,
  email: ContactPreview,
  phone: ContactPreview,
  color: ColorPreview,
  text: TextPreview,
  code: TextPreview,
  empty: EmptyPreview
}
