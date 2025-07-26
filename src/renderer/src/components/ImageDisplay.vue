<template>
  <img
    v-if="imageSrc"
    :src="imageSrc"
    :alt="alt"
    :class="imageClass"
    @load="handleLoad"
    @error="handleError"
  />
  <div v-else-if="isLoading" :class="loadingClass">
    Loading...
  </div>
  <div v-else-if="hasError" :class="errorClass">
    {{ errorMessage }}
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onUnmounted, watch, nextTick } from 'vue'

interface Props {
  imageData: Uint8Array
  alt?: string
  imageClass?: string
  loadingClass?: string
  errorClass?: string
  sizeThreshold?: number // 大小阈值，单位字节，默认100KB
}

const props = withDefaults(defineProps<Props>(), {
  alt: 'Image preview',
  imageClass: '',
  loadingClass: 'image-loading',
  errorClass: 'image-error',
  sizeThreshold: 100 * 1024 // 100KB
})

const emit = defineEmits<{
  load: [event: Event]
  error: [event: Event]
}>()

// 状态管理
const imageSrc = ref<string>('')
const isLoading = ref<boolean>(true)
const hasError = ref<boolean>(false)
const errorMessage = ref<string>('')
const currentObjectURL = ref<string | null>(null)

// 判断是否为大图片
const isLargeImage = computed(() => {
  return props.imageData && props.imageData.length > props.sizeThreshold
})

// 处理图片加载
const processImage = async () => {
  if (!props.imageData || props.imageData.length === 0) {
    hasError.value = true
    errorMessage.value = '无效的图片数据'
    isLoading.value = false
    return
  }

  try {
    isLoading.value = true
    hasError.value = false
    errorMessage.value = ''

    // 清理之前的 Object URL
    cleanupObjectURL()

    if (isLargeImage.value) {
      // 大图片使用 Blob URL
      const blob = new Blob([props.imageData], { type: 'image/png' })
      const objectURL = URL.createObjectURL(blob)
      currentObjectURL.value = objectURL
      imageSrc.value = objectURL
    } else {
      // 小图片使用 base64
      try {
        const binaryString = Array.from(props.imageData)
          .map(byte => String.fromCharCode(byte))
          .join('')
        const base64String = btoa(binaryString)
        imageSrc.value = `data:image/png;base64,${base64String}`
      } catch (error) {
        console.error('转换 base64 失败:', error)
        // 回退到 Blob URL 方案
        const blob = new Blob([props.imageData], { type: 'image/png' })
        const objectURL = URL.createObjectURL(blob)
        currentObjectURL.value = objectURL
        imageSrc.value = objectURL
      }
    }

    // 等待下一帧确保 DOM 更新
    await nextTick()
    isLoading.value = false
  } catch (error) {
    console.error('处理图片失败:', error)
    hasError.value = true
    errorMessage.value = '图片处理失败'
    isLoading.value = false
  }
}

// 清理 Object URL
const cleanupObjectURL = () => {
  if (currentObjectURL.value) {
    URL.revokeObjectURL(currentObjectURL.value)
    currentObjectURL.value = null
  }
}

// 处理图片加载成功
const handleLoad = (event: Event) => {
  isLoading.value = false
  hasError.value = false
  
  // 如果使用的是 Object URL，可以在这里清理（可选）
  // 注意：立即清理可能导致图片无法显示，所以这里注释掉
  // if (isLargeImage.value) {
  //   cleanupObjectURL()
  // }
  
  emit('load', event)
}

// 处理图片加载失败
const handleError = (event: Event) => {
  isLoading.value = false
  hasError.value = true
  errorMessage.value = '图片加载失败'
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
  min-height: 100px;
  color: #64748b;
  font-size: 14px;
  background: linear-gradient(135deg, #f8fafc 0%, #f1f5f9 100%);
  border-radius: 8px;
}

.image-error {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 100px;
  color: #ef4444;
  font-size: 14px;
  background: linear-gradient(135deg, #fef2f2 0%, #fee2e2 100%);
  border-radius: 8px;
  text-align: center;
  padding: 16px;
}
</style> 