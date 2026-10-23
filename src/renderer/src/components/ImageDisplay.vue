<template>
  <img
    v-if="imageSrc"
    :src="imageSrc"
    :alt="alt"
    :class="imageClass"
    decoding="async"
    @load="handleLoad"
    @error="handleError"
  />
  <div v-else-if="hasError" :class="errorClass">
    {{ PREVIEW_LABELS.imageUnavailable }}
  </div>
</template>

<script setup lang="ts">
import { ref, onUnmounted, watch } from 'vue'
import { acquireClipboardImageURL } from '@renderer/utils/clipboard-image-urls'
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'

interface Props {
  imageData: Uint8Array
  alt?: string
  imageClass?: string
  errorClass?: string
}

const props = withDefaults(defineProps<Props>(), {
  alt: PREVIEW_LABELS.image,
  imageClass: '',
  errorClass: 'image-error'
})

const emit = defineEmits<{
  load: [event: Event]
  error: [event: Event]
}>()

// 状态管理
const imageSrc = ref<string>('')
const hasError = ref<boolean>(false)
let imageURL: ReturnType<typeof acquireClipboardImageURL> | undefined

// Blob URL 避免 Array.from/map/join/base64 的多次分配与主线程编码。
const processImage = (): void => {
  cleanupObjectURL()
  imageSrc.value = ''
  hasError.value = false
  try {
    imageURL = acquireClipboardImageURL(props.imageData)
    imageSrc.value = imageURL.url
  } catch (error) {
    hasError.value = true
    console.warn('剪贴板图片预览失败:', error)
  }
}

// 清理 Object URL
const cleanupObjectURL = (): void => {
  imageURL?.release()
  imageURL = undefined
}

// 处理图片加载成功
const handleLoad = (event: Event): void => {
  hasError.value = false

  emit('load', event)
}

// 处理图片加载失败
const handleError = (event: Event): void => {
  hasError.value = true
  imageSrc.value = ''
  cleanupObjectURL()
  emit('error', event)
}

// 监听图片数据变化
watch(
  () => props.imageData,
  () => {
    processImage()
  },
  { immediate: true }
)

// 组件卸载时清理资源
onUnmounted(() => {
  cleanupObjectURL()
})
</script>

<style scoped>
.image-error {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 0;
  height: 100%;
  width: 100%;
  box-sizing: border-box;
  color: var(--color-danger);
  font-size: 14px;
  background: var(--color-danger-bg);
  border-radius: 8px;
  text-align: center;
  padding: 16px;
}
</style>
