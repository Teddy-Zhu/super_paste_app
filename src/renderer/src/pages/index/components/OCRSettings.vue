<script setup lang="ts">
import type { SettingsStore } from '@renderer/stores/common'
import { invoke, onSimple } from '@renderer/api/ipc'
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import {
  DEFAULT_OCR_MODEL_ID,
  OCR_MODELS,
  OCR_MODEL_STAGE_LABELS,
  type OCRModelId,
  type OCRModelStatus,
  type OCRSettings
} from '../../../../../shared/ocr-models'

const props = defineProps<{ settingsStore: SettingsStore }>()

const readStoreConfig = (): OCRSettings => ({
  enableLocalOCR: Boolean(props.settingsStore.enableLocalOCR),
  ocrMode: props.settingsStore.ocrMode === 'remote' ? 'remote' : 'local',
  ocrLocalModel: OCR_MODELS.some((model) => model.id === props.settingsStore.ocrLocalModel)
    ? props.settingsStore.ocrLocalModel
    : DEFAULT_OCR_MODEL_ID,
  ocrRemoteUrl: String(props.settingsStore.ocrRemoteUrl || ''),
  ocrRemoteExtraParams: String(props.settingsStore.ocrRemoteExtraParams || '{}'),
  ocrRemoteImageFieldName: String(props.settingsStore.ocrRemoteImageFieldName || 'image'),
  ocrRemoteRequestFormat: props.settingsStore.ocrRemoteRequestFormat === 'form' ? 'form' : 'json',
  ocrRemoteResultPath: String(props.settingsStore.ocrRemoteResultPath || 'data.result')
})

const normalize = (config: OCRSettings): OCRSettings => ({
  ...config,
  ocrRemoteUrl: config.ocrRemoteUrl.trim(),
  ocrRemoteExtraParams: config.ocrRemoteExtraParams.trim() || '{}',
  ocrRemoteImageFieldName: config.ocrRemoteImageFieldName.trim() || 'image',
  ocrRemoteResultPath: config.ocrRemoteResultPath.trim() || 'data.result'
})
const form = reactive<OCRSettings>(readStoreConfig())
const saveMessage = ref('')
const saveError = ref('')
const isSaving = ref(false)
const downloadingId = ref<OCRModelId | null>(null)
const modelStatuses = ref<Partial<Record<OCRModelId, OCRModelStatus>>>({})
const savedSnapshot = ref(JSON.stringify(normalize(form)))
let unsubscribe: (() => void) | undefined

const hasUnsavedChanges = computed(() => JSON.stringify(normalize(form)) !== savedSnapshot.value)
const selectedModel = computed(
  () => OCR_MODELS.find((model) => model.id === form.ocrLocalModel) || OCR_MODELS[0]
)
const selectedStatus = computed(() => modelStatuses.value[form.ocrLocalModel])
const modelGroups = [...new Set(OCR_MODELS.map((model) => model.group))]
const isDownloading = computed(() => selectedStatus.value?.state === 'downloading')
const downloadProgress = computed(() => {
  const status = selectedStatus.value
  return status?.totalBytes
    ? Math.min(100, Math.floor((status.receivedBytes / status.totalBytes) * 100))
    : 0
})
const modelStatusText = computed(() => {
  const status = selectedStatus.value
  if (status?.state === 'ready') return '已下载，可离线使用'
  if (status?.state === 'error') return status.error || '下载失败'
  if (status?.state === 'downloading') return OCR_MODEL_STAGE_LABELS[status.stage || 'downloading']
  return '尚未下载'
})
const formatSize = (bytes: number): string => (bytes / 1024 / 1024).toFixed(1) + ' MB'

const loadFromStore = (): void => {
  Object.assign(form, readStoreConfig())
  savedSnapshot.value = JSON.stringify(normalize(form))
}

const extraParamsError = computed(() => {
  if (form.ocrMode !== 'remote') return ''
  try {
    const params: unknown = JSON.parse(form.ocrRemoteExtraParams.trim() || '{}')
    return params && typeof params === 'object' && !Array.isArray(params)
      ? ''
      : '额外参数必须是 JSON 对象'
  } catch {
    return '额外参数 JSON 格式不正确'
  }
})

const downloadModel = async (): Promise<void> => {
  const id = form.ocrLocalModel
  downloadingId.value = id
  saveError.value = ''
  saveMessage.value = ''
  try {
    const result = await invoke('ocr:download-model', id)
    if (!result.success) throw new Error(result.error || '模型下载失败')
  } catch (error) {
    saveError.value = error instanceof Error ? error.message : String(error)
  } finally {
    downloadingId.value = null
  }
}

const cancelDownload = async (): Promise<void> => {
  try {
    const result = await invoke('ocr:cancel-download', form.ocrLocalModel)
    if (!result.success) throw new Error(result.error || '取消下载失败')
  } catch (error) {
    saveError.value = error instanceof Error ? error.message : String(error)
  }
}

const saveConfig = async (): Promise<void> => {
  saveMessage.value = ''
  saveError.value = ''
  if (extraParamsError.value) {
    saveError.value = extraParamsError.value
    return
  }
  if (form.ocrMode === 'remote' && form.enableLocalOCR && !form.ocrRemoteUrl.trim()) {
    saveError.value = '启用远程 OCR 时，调用地址不能为空'
    return
  }
  isSaving.value = true
  try {
    const result = await invoke('ocr:save-settings', normalize(form))
    if (!result.success || !result.data) throw new Error(result.error || 'OCR 配置保存失败')
    // 配置已由主进程统一保存，前端只同步状态。
    for (const [key, value] of Object.entries(result.data))
      props.settingsStore.$setSilent(key, value)
    Object.assign(form, result.data)
    savedSnapshot.value = JSON.stringify(normalize(form))
    saveMessage.value = 'OCR 配置已保存'
  } catch (error) {
    saveError.value = error instanceof Error ? error.message : String(error)
  } finally {
    isSaving.value = false
  }
}

const resetUnsaved = (): void => {
  saveMessage.value = ''
  saveError.value = ''
  loadFromStore()
}

onMounted(async () => {
  unsubscribe = onSimple('ocr:model-status', (status: OCRModelStatus) => {
    modelStatuses.value[status.id] = status
  })
  try {
    const result = await invoke('ocr:list-models')
    if (!result.success || !result.data) throw new Error(result.error || '无法读取模型状态')
    for (const status of result.data) {
      if (!modelStatuses.value[status.id]) modelStatuses.value[status.id] = status
    }
  } catch (error) {
    saveError.value = error instanceof Error ? error.message : String(error)
  }
})
onBeforeUnmount(() => unsubscribe?.())
watch(readStoreConfig, () => {
  if (!isSaving.value && !hasUnsavedChanges.value) loadFromStore()
})
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">OCR 设置</h3>
      <p class="section-subtitle">选择本地识别模型或配置远程 OCR 服务</p>
    </div>

    <div class="settings-grid">
      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">自动 OCR</h4>
          <p class="setting-description">保存图片到历史记录时自动执行 OCR 识别</p>
        </div>
        <div class="setting-control">
          <label class="toggle-switch">
            <input v-model="form.enableLocalOCR" type="checkbox" :disabled="isSaving" />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <div class="setting-card">
        <div class="setting-info">
          <h4 class="setting-title">OCR 方式</h4>
          <p class="setting-description">本地识别支持离线使用，远程识别调用 HTTP 接口</p>
        </div>
        <div class="setting-control ocr-mode-control">
          <select
            v-model="form.ocrMode"
            class="select-input"
            :disabled="isSaving || !!downloadingId"
          >
            <option value="local">本地模型</option>
            <option value="remote">远程接口</option>
          </select>
        </div>
      </div>

      <div v-if="form.ocrMode === 'local'" class="setting-card multi-line-card">
        <div class="setting-info full-width">
          <h4 class="setting-title">本地 OCR 模型</h4>
          <p class="setting-description">
            启用自动 OCR 或首次手动识别时下载所选模型，下载后可离线使用。
          </p>
          <select
            v-model="form.ocrLocalModel"
            class="select-input"
            :disabled="isSaving || !!downloadingId"
          >
            <optgroup v-for="group in modelGroups" :key="group" :label="group">
              <option
                v-for="model in OCR_MODELS.filter((item) => item.group === group)"
                :key="model.id"
                :value="model.id"
              >
                {{ model.name }} · {{ formatSize(model.size) }}
              </option>
            </optgroup>
          </select>
          <p class="model-description">
            {{ selectedModel.description }} · 下载大小 {{ formatSize(selectedModel.size) }}
          </p>
          <p
            class="model-status"
            :class="{ 'field-error': selectedStatus?.state === 'error' }"
            role="status"
          >
            {{ modelStatusText }}
          </p>
          <div v-if="isDownloading" class="download-progress">
            <progress :value="downloadProgress" max="100" aria-label="模型下载进度"></progress>
            <span
              >{{ downloadProgress }}% · {{ formatSize(selectedStatus?.receivedBytes || 0) }} /
              {{ formatSize(selectedStatus?.totalBytes || selectedModel.size) }}</span
            >
          </div>
          <div class="save-actions">
            <button
              v-if="selectedStatus?.state !== 'ready' && !isDownloading"
              class="shortcut-button secondary"
              :disabled="isSaving || !!downloadingId"
              @click="downloadModel"
            >
              {{ selectedStatus?.state === 'error' ? '重试下载' : '下载模型' }}
            </button>
            <button v-if="isDownloading" class="shortcut-button secondary" @click="cancelDownload">
              取消下载
            </button>
          </div>
        </div>
      </div>

      <template v-if="form.ocrMode === 'remote'">
        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <h4 class="setting-title">调用地址</h4>
            <p class="setting-description">远程 OCR API 地址，默认使用 POST 请求</p>
            <input
              v-model.trim="form.ocrRemoteUrl"
              :disabled="isSaving"
              class="text-input"
              type="text"
              placeholder="https://example.com/api/ocr"
            />
          </div>
        </div>

        <div class="setting-card">
          <div class="setting-info">
            <h4 class="setting-title">请求格式</h4>
            <p class="setting-description">json: base64 字段；form: 文件字段上传</p>
          </div>
          <div class="setting-control ocr-mode-control">
            <select v-model="form.ocrRemoteRequestFormat" :disabled="isSaving" class="select-input">
              <option value="json">JSON</option>
              <option value="form">表单 (multipart/form-data)</option>
            </select>
          </div>
        </div>

        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <h4 class="setting-title">图片字段名称</h4>
            <p class="setting-description">
              例如 `image`，JSON 模式下为 base64 字段，表单模式下为文件字段
            </p>
            <input
              v-model.trim="form.ocrRemoteImageFieldName"
              :disabled="isSaving"
              class="text-input"
              type="text"
              placeholder="image"
            />
          </div>
        </div>

        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <h4 class="setting-title">额外调用参数 (JSON)</h4>
            <p class="setting-description">仅支持 JSON 对象，所有字段会与图片字段一起提交</p>
            <textarea
              v-model="form.ocrRemoteExtraParams"
              :disabled="isSaving"
              class="text-area"
              rows="5"
              placeholder='{"lang":"zh","scene":"general"}'
            ></textarea>
            <p v-if="extraParamsError" class="field-error">{{ extraParamsError }}</p>
          </div>
        </div>

        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <h4 class="setting-title">返回结果读取路径</h4>
            <p class="setting-description">
              仅支持 JSON 响应。示例：`data.result` 对应 `res.data.result`
            </p>
            <input
              v-model.trim="form.ocrRemoteResultPath"
              :disabled="isSaving"
              class="text-input"
              type="text"
              placeholder="data.result"
            />
          </div>
        </div>
      </template>

      <div class="setting-card multi-line-card">
        <div class="setting-info full-width">
          <h4 class="setting-title">保存配置</h4>
          <p class="setting-description">修改后点击保存，配置才会写入并生效</p>
          <div class="save-actions">
            <button
              class="shortcut-button primary"
              :disabled="!hasUnsavedChanges || isSaving || !!downloadingId"
              @click="saveConfig"
            >
              {{ isSaving ? '正在保存…' : '保存 OCR 配置' }}
            </button>
            <button
              class="shortcut-button secondary"
              :disabled="!hasUnsavedChanges || isSaving || !!downloadingId"
              @click="resetUnsaved"
            >
              放弃修改
            </button>
          </div>
          <p v-if="hasUnsavedChanges" class="unsaved-text">有未保存修改</p>
          <p v-if="saveMessage" class="save-message">{{ saveMessage }}</p>
          <p v-if="saveError" class="field-error">{{ saveError }}</p>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.multi-line-card {
  align-items: flex-start;
}

.full-width {
  width: 100%;
}

.ocr-mode-control {
  min-width: 220px;
}

.select-input,
.text-input,
.text-area {
  width: 100%;
  margin-top: 12px;
  padding: 10px 12px;
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 8px;
  font-size: 14px;
  background: #fff;
  transition: all 0.2s ease;
  box-sizing: border-box;
}

.text-area {
  resize: vertical;
  min-height: 110px;
  font-family:
    ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, 'Liberation Mono', 'Courier New',
    monospace;
  line-height: 1.4;
}

.select-input:focus,
.text-input:focus,
.text-area:focus {
  outline: none;
  border-color: #007aff;
  box-shadow: 0 0 0 3px rgba(0, 122, 255, 0.1);
}

.field-error {
  margin-top: 8px;
  font-size: 12px;
  color: #d70015;
}

.save-actions {
  display: flex;
  gap: 12px;
  margin-top: 12px;
}

.shortcut-button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.unsaved-text {
  margin-top: 10px;
  font-size: 12px;
  color: #b26a00;
}

.save-message {
  margin-top: 10px;
  font-size: 12px;
  color: #1a7f37;
}

.model-description,
.model-status {
  margin-top: 10px;
  font-size: 12px;
  color: #666;
}
.model-status.field-error {
  color: #d70015;
}
.download-progress {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 12px;
  font-size: 12px;
  color: #666;
}
.download-progress progress {
  width: 100%;
  height: 8px;
  accent-color: #007aff;
}
</style>
