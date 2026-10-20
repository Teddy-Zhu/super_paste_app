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
  <div v-else-if="isLoading" :class="loadingClass">{{ PREVIEW_LABELS.imageLoading }}</div>
  <div v-else-if="hasError" :class="errorClass">
    {{ PREVIEW_LABELS.imageUnavailable }}
  </div>
</template>

<script setup lang="ts">
import { ref, onUnmounted, watch } from 'vue'
import { getImageMimeType } from '@renderer/utils/clipboard-preview'
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'

interface Props {
  imageData: Uint8Array
  alt?: string
  imageClass?: string
  loadingClass?: string
  errorClass?: string
}

const props = withDefaults(defineProps<Props>(), {
  alt: PREVIEW_LABELS.image,
  imageClass: '',
  loadingClass: 'image-loading',
  errorClass: 'image-error'
})

const emit = defineEmits<{
  load: [event: Event]
  error: [event: Event]
}>()

// 状态管理
const imageSrc = ref<string>('')
const isLoading = ref<boolean>(true)
const hasError = ref<boolean>(false)
const currentObjectURL = ref<string | null>(null)

// 将 Uint8Array 安全转换为 ArrayBuffer，避免 Blob 构造函数类型报错
const toArrayBuffer = (data: Uint8Array): ArrayBuffer => {
  const buffer = new ArrayBuffer(data.byteLength)
  new Uint8Array(buffer).set(data)
  return buffer
}

// Blob URL 避免 Array.from/map/join/base64 的多次分配与主线程编码。
const processImage = (): void => {
  cleanupObjectURL()
  imageSrc.value = ''
  hasError.value = false
  isLoading.value = true
  try {
    if (!props.imageData?.byteLength) throw new Error('无效的图片数据')
    const blob = new Blob([toArrayBuffer(props.imageData)], {
      type: getImageMimeType(props.imageData)
    })
    currentObjectURL.value = URL.createObjectURL(blob)
    imageSrc.value = currentObjectURL.value
  } catch (error) {
    hasError.value = true
    console.warn('剪贴板图片预览失败:', error)
    isLoading.value = false
  }
}

// 清理 Object URL
const cleanupObjectURL = (): void => {
  if (currentObjectURL.value) {
    URL.revokeObjectURL(currentObjectURL.value)
    currentObjectURL.value = null
  }
}

// 处理图片加载成功
const handleLoad = (event: Event): void => {
  isLoading.value = false
  hasError.value = false

  emit('load', event)
}

// 处理图片加载失败
const handleError = (event: Event): void => {
  isLoading.value = false
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
.image-loading {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 0;
  height: 100%;
  width: 100%;
  box-sizing: border-box;
  color: #64748b;
  font-size: 14px;
  background: linear-gradient(135deg, #f8fafc 0%, #f1f5f9 100%);
  border-radius: 8px;
}

.image-error {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 0;
  height: 100%;
  width: 100%;
  box-sizing: border-box;
  color: #ef4444;
  font-size: 14px;
  background: linear-gradient(135deg, #fef2f2 0%, #fee2e2 100%);
  border-radius: 8px;
  text-align: center;
  padding: 16px;
}
</style>
