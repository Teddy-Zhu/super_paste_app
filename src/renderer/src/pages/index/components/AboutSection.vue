<script setup lang="ts">
import { getDesktopPlatform } from '@renderer/api/desktop'

const { appName, version } = getDesktopPlatform()
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">关于 {{ appName }}</h3>
      <p class="section-subtitle">应用信息和版本详情</p>
    </div>

    <div class="about-content">
      <div class="app-info">
        <h4 class="app-name">{{ appName }}</h4>
        <p class="app-version">版本 {{ version }}</p>
        <p class="app-description">智能剪贴板与 OCR 工具</p>
      </div>
    </div>
  </div>
</template>
