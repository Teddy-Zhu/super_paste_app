<script setup lang="ts">
import IconAt from '~icons/mdi/at'
import IconPhone from '~icons/mdi/phone-outline'
import type { PreviewProps, PreviewEvents } from '../types'

withDefaults(defineProps<PreviewProps>(), { expanded: false, plain: false })
defineEmits<PreviewEvents>()
</script>

<template>
  <div class="contact-icon"><IconAt v-if="preview.kind === 'email'" /><IconPhone v-else /></div>
  <p class="contact-value" :class="{ expanded }" dir="auto">{{ preview.text.trim() }}</p>
</template>

<style scoped>
.contact-icon {
  font-size: 40px;
  color: var(--color-type-red);
  margin: auto auto 0;
}
.contact-value {
  font-size: 21px;
  font-weight: 500;
  margin: 0 0 auto;
  text-align: center;
  overflow-wrap: anywhere;
}
.contact-value.expanded {
  margin: 0;
}
</style>
