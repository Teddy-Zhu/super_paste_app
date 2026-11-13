<template>
  <img
    v-if="imageSrc && !hasError"
    :src="imageSrc"
    :alt="alt"
    :class="[imageClass, { 'image-pending': !ready }]"
    decoding="async"
    @load="handleLoad"
    @error="handleError"
  />
  <div v-else-if="hasError" :class="errorClass">
    {{ PREVIEW_LABELS.imageUnavailable }}
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { PREVIEW_LABELS } from '@renderer/constants/clipboard-preview'

interface Props {
  imageSrc: string
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

const hasError = ref<boolean>(false)
const ready = ref(false)
let requestVersion = 0

// 使用同一个图片节点读取和解码，完整可绘制后直接显示，不展示渐进绘制过程。
const handleLoad = async (event: Event): Promise<void> => {
  const image = event.target as HTMLImageElement
  const version = requestVersion
  const isCurrent = (): boolean =>
    version === requestVersion && image.getAttribute('src') === props.imageSrc
  try {
    await image.decode()
  } catch {
    if (isCurrent()) handleError(event)
    return
  }
  if (!isCurrent()) return

  ready.value = true
  emit('load', event)
}

// 处理图片加载失败
const handleError = (event: Event): void => {
  ready.value = false
  hasError.value = true
  emit('error', event)
}

watch(
  () => props.imageSrc,
  () => {
    requestVersion++
    ready.value = false
    hasError.value = false
  },
  { flush: 'sync' }
)

onBeforeUnmount(() => requestVersion++)
</script>

<style scoped>
.image-pending {
  visibility: hidden;
}
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
