<script setup lang="ts">
import { useSettingsStore } from '@renderer/stores/common'
import { invoke, onSimple } from '@renderer/api/ipc'
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { onClickOutside } from '@vueuse/core'
import IconChevronDown from '~icons/mdi/chevron-down'
import { AI_OCR_TEXT, OCR_MANAGE_TEXT } from '@renderer/constants/ocr'
import {
  createDefaultAIOCRConfigs,
  isAIOCRProvider,
  validateAIOCRConfig,
  type AIOCRConfigs,
  type AIOCRModelOption,
  type AIOCRAPIRequest
} from '@shared/ai-ocr'
import {
  DEFAULT_OCR_MODEL_ID,
  OCR_MODELS,
  OCR_MODEL_STAGE_LABELS,
  type OCRModelId,
  type OCRModelStatus,
  type OCRSettings
} from '@shared/ocr-models'

const settingsStore = useSettingsStore()

const readStoreConfig = (): OCRSettings => ({
  enableLocalOCR: Boolean(settingsStore.enableLocalOCR),
  ocrMode: settingsStore.ocrMode,
  ocrLocalModel: OCR_MODELS.some((model) => model.id === settingsStore.ocrLocalModel)
    ? settingsStore.ocrLocalModel
    : DEFAULT_OCR_MODEL_ID,
  ocrRemoteUrl: String(settingsStore.ocrRemoteUrl || ''),
  ocrRemoteExtraParams: String(settingsStore.ocrRemoteExtraParams || '{}'),
  ocrRemoteImageFieldName: String(settingsStore.ocrRemoteImageFieldName || 'image'),
  ocrRemoteRequestFormat: settingsStore.ocrRemoteRequestFormat === 'form' ? 'form' : 'json',
  ocrRemoteResultPath: String(settingsStore.ocrRemoteResultPath || 'data.result'),
  ocrAIConfigs: normalizeAIConfigs(settingsStore.ocrAIConfigs)
})

const normalizeAIConfigs = (configs: AIOCRConfigs): AIOCRConfigs => {
  const defaults = createDefaultAIOCRConfigs()
  for (const provider of ['openai', 'claude'] as const) {
    const config = { ...defaults[provider], ...configs?.[provider] }
    defaults[provider] = {
      baseUrl: config.baseUrl.trim(),
      apiKey: config.apiKey.trim(),
      model: config.model.trim(),
      api: config.api === 'responses' ? 'responses' : 'chat'
    }
  }
  return defaults
}

const normalize = (config: OCRSettings): OCRSettings => ({
  ...config,
  ocrRemoteUrl: config.ocrRemoteUrl.trim(),
  ocrRemoteExtraParams: config.ocrRemoteExtraParams.trim() || '{}',
  ocrRemoteImageFieldName: config.ocrRemoteImageFieldName.trim() || 'image',
  ocrRemoteResultPath: config.ocrRemoteResultPath.trim() || 'data.result',
  ocrAIConfigs: normalizeAIConfigs(config.ocrAIConfigs)
})
const form = reactive<OCRSettings>(readStoreConfig())
const saveMessage = ref('')
const saveError = ref('')
const isSaving = ref(false)
const downloadingId = ref<OCRModelId | null>(null)
const maintaining = ref(false)
const confirmDelete = ref(false)
const models = ref<AIOCRModelOption[]>([])
const modelPicker = ref<HTMLElement | null>(null)
const modelInput = ref<HTMLInputElement | null>(null)
const modelList = ref<HTMLElement | null>(null)
const modelListOpen = ref(false)
const activeModelIndex = ref(-1)
const apiMessage = ref('')
const apiError = ref('')
const fetchingModels = ref(false)
const testingConnection = ref(false)
let profileVersion = 0
let testVersion = 0
let disposed = false
const modelStatuses = ref<Partial<Record<OCRModelId, OCRModelStatus>>>({})
const savedSnapshot = ref(JSON.stringify(normalize(form)))
let unsubscribe: (() => void) | undefined

const hasUnsavedChanges = computed(() => JSON.stringify(normalize(form)) !== savedSnapshot.value)
const selectedModel = computed(
  () => OCR_MODELS.find((model) => model.id === form.ocrLocalModel) || OCR_MODELS[0]
)
const selectedStatus = computed(() => modelStatuses.value[form.ocrLocalModel])
const selectedAIConfig = computed(() =>
  isAIOCRProvider(form.ocrMode) ? form.ocrAIConfigs[form.ocrMode] : null
)
const revealActiveModel = (): void => {
  void nextTick(() => {
    const option =
      modelList.value?.querySelectorAll<HTMLElement>('[role="option"]')[activeModelIndex.value]
    option?.scrollIntoView({ block: 'nearest' })
  })
}
const closeModelList = (): void => {
  modelListOpen.value = false
  activeModelIndex.value = -1
}
const openModelList = (): void => {
  if (isSaving.value || !models.value.length) return
  // 输入值只用于标记当前选项，始终展示接口返回的完整列表。
  activeModelIndex.value = Math.max(
    0,
    models.value.findIndex((model) => model.id === selectedAIConfig.value?.model)
  )
  modelListOpen.value = true
  revealActiveModel()
}
const toggleModelList = (): void => {
  if (modelListOpen.value) closeModelList()
  else {
    openModelList()
    modelInput.value?.focus({ preventScroll: true })
  }
}
const selectAIModel = (model: AIOCRModelOption): void => {
  if (!selectedAIConfig.value || isSaving.value) return
  selectedAIConfig.value.model = model.id
  closeModelList()
  modelInput.value?.focus({ preventScroll: true })
}
const handleModelKeydown = (event: KeyboardEvent): void => {
  if (event.isComposing) return
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    if (!models.value.length) return
    event.preventDefault()
    if (!modelListOpen.value) openModelList()
    else {
      const direction = event.key === 'ArrowDown' ? 1 : -1
      activeModelIndex.value =
        (activeModelIndex.value + direction + models.value.length) % models.value.length
      revealActiveModel()
    }
  } else if (event.key === 'Enter' && modelListOpen.value) {
    event.preventDefault()
    const model = models.value[activeModelIndex.value]
    if (model) selectAIModel(model)
  } else if (event.key === 'Escape' && modelListOpen.value) {
    event.preventDefault()
    event.stopPropagation()
    closeModelList()
  } else if (event.key === 'Tab') closeModelList()
}
const handleModelFocusOut = (event: FocusEvent): void => {
  if (!modelPicker.value?.contains(event.relatedTarget as Node | null)) closeModelList()
}
onClickOutside(modelPicker, closeModelList)
watch(isSaving, closeModelList)
const modelGroups = [...new Set(OCR_MODELS.map((model) => model.group))]
const isDownloading = computed(() => selectedStatus.value?.state === 'downloading')
const localBusy = computed(() => !!downloadingId.value || maintaining.value)
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

const maintainModel = async (action: 'delete' | 'redownload'): Promise<void> => {
  const id = form.ocrLocalModel
  maintaining.value = true
  confirmDelete.value = false
  saveError.value = ''
  saveMessage.value = ''
  try {
    const result = await invoke(
      action === 'delete' ? 'ocr:delete-model' : 'ocr:redownload-model',
      id
    )
    if (!result.success) throw new Error(result.error || OCR_MANAGE_TEXT.modelActionFailed)
    if (
      action === 'delete' &&
      settingsStore.ocrMode === 'local' &&
      settingsStore.ocrLocalModel === id
    ) {
      form.enableLocalOCR = false
      const saved = JSON.parse(savedSnapshot.value) as OCRSettings
      saved.enableLocalOCR = false
      savedSnapshot.value = JSON.stringify(saved)
    }
    saveMessage.value = action === 'delete' ? OCR_MANAGE_TEXT.removed : OCR_MANAGE_TEXT.redownloaded
  } catch (error) {
    saveError.value = error instanceof Error ? error.message : String(error)
  } finally {
    maintaining.value = false
  }
  try {
    const result = await invoke('ocr:list-models')
    if (!result.success || !result.data)
      throw new Error(result.error || OCR_MANAGE_TEXT.modelActionFailed)
    for (const status of result.data) modelStatuses.value[status.id] = status
  } catch (error) {
    saveError.value = error instanceof Error ? error.message : String(error)
  }
}

const apiRequest = (): AIOCRAPIRequest => {
  if (!isAIOCRProvider(form.ocrMode)) throw new Error('AI OCR 配置无效')
  return { provider: form.ocrMode, config: { ...form.ocrAIConfigs[form.ocrMode] } }
}

const fetchModels = async (): Promise<void> => {
  const version = profileVersion
  fetchingModels.value = true
  apiError.value = ''
  apiMessage.value = ''
  try {
    const result = await invoke('ocr:ai-list-models', apiRequest())
    if (disposed || version !== profileVersion) return
    if (!result.success || !result.data) throw new Error(result.error || OCR_MANAGE_TEXT.listFailed)
    models.value = result.data
    if (result.data.length) openModelList()
    else {
      closeModelList()
      apiMessage.value = OCR_MANAGE_TEXT.noModels
    }
  } catch (error) {
    if (!disposed && version === profileVersion)
      apiError.value = error instanceof Error ? error.message : String(error)
  } finally {
    if (version === profileVersion) fetchingModels.value = false
  }
}

const testConnection = async (): Promise<void> => {
  const version = testVersion
  testingConnection.value = true
  apiError.value = ''
  apiMessage.value = ''
  try {
    const result = await invoke('ocr:ai-test-connection', apiRequest())
    if (disposed || version !== testVersion) return
    if (!result.success || !result.data) throw new Error(result.error || OCR_MANAGE_TEXT.testFailed)
    apiMessage.value = `${OCR_MANAGE_TEXT.connected} · ${result.data.model} · ${result.data.latencyMs} ms`
  } catch (error) {
    if (!disposed && version === testVersion)
      apiError.value = error instanceof Error ? error.message : String(error)
  } finally {
    if (version === testVersion) testingConnection.value = false
  }
}

watch(
  () => [form.ocrMode, selectedAIConfig.value?.baseUrl, selectedAIConfig.value?.apiKey],
  () => {
    profileVersion++
    models.value = []
    closeModelList()
    fetchingModels.value = false
    apiError.value = ''
    apiMessage.value = ''
  }
)
watch(
  () => [
    form.ocrMode,
    selectedAIConfig.value?.baseUrl,
    selectedAIConfig.value?.apiKey,
    selectedAIConfig.value?.model,
    selectedAIConfig.value?.api
  ],
  () => {
    testVersion++
    testingConnection.value = false
    apiError.value = ''
    apiMessage.value = ''
  }
)
watch(
  () => form.ocrLocalModel,
  () => {
    confirmDelete.value = false
  }
)

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
  if (isAIOCRProvider(form.ocrMode)) {
    try {
      validateAIOCRConfig(form.ocrMode, form.ocrAIConfigs[form.ocrMode], form.enableLocalOCR)
    } catch (error) {
      saveError.value = error instanceof Error ? error.message : String(error)
      return
    }
  }
  isSaving.value = true
  try {
    settingsStore.$patch(normalize(form))
    await settingsStore.$whenSynced()
    loadFromStore()
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
onBeforeUnmount(() => {
  disposed = true
  unsubscribe?.()
})
watch(readStoreConfig, () => {
  if (!isSaving.value && !hasUnsavedChanges.value) loadFromStore()
})
</script>

<template>
  <div class="content-section">
    <div class="section-header">
      <h3 class="section-title">OCR 设置</h3>
      <p class="section-subtitle">{{ AI_OCR_TEXT.subtitle }}</p>
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
          <p class="setting-description">{{ AI_OCR_TEXT.modeDescription }}</p>
        </div>
        <div class="setting-control ocr-mode-control">
          <select v-model="form.ocrMode" class="select-input" :disabled="isSaving || localBusy">
            <option value="local">本地模型</option>
            <option value="remote">远程接口</option>
            <option value="openai">{{ AI_OCR_TEXT.openai }}</option>
            <option value="claude">{{ AI_OCR_TEXT.claude }}</option>
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
            :disabled="isSaving || localBusy"
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
            <template v-if="selectedStatus?.state === 'ready' && !isDownloading">
              <button
                class="shortcut-button secondary"
                :disabled="isSaving || localBusy"
                @click="maintainModel('redownload')"
              >
                {{ maintaining ? OCR_MANAGE_TEXT.working : OCR_MANAGE_TEXT.redownload }}
              </button>
              <button
                class="shortcut-button secondary"
                :disabled="isSaving || localBusy"
                @click="confirmDelete = true"
              >
                {{ OCR_MANAGE_TEXT.remove }}
              </button>
            </template>
            <button
              v-if="selectedStatus?.state !== 'ready' && !isDownloading"
              class="shortcut-button secondary"
              :disabled="isSaving || localBusy"
              @click="downloadModel"
            >
              {{ selectedStatus?.state === 'error' ? '重试下载' : '下载模型' }}
            </button>
            <button v-if="isDownloading" class="shortcut-button secondary" @click="cancelDownload">
              取消下载
            </button>
          </div>
          <div v-if="confirmDelete" class="model-status" role="alert">
            <p>{{ OCR_MANAGE_TEXT.removeConfirm }}</p>
            <div class="save-actions">
              <button
                class="shortcut-button secondary"
                :disabled="localBusy"
                @click="maintainModel('delete')"
              >
                {{ OCR_MANAGE_TEXT.confirmRemove }}
              </button>
              <button class="shortcut-button secondary" @click="confirmDelete = false">
                {{ OCR_MANAGE_TEXT.cancel }}
              </button>
            </div>
          </div>
        </div>
      </div>

      <template v-if="selectedAIConfig">
        <div v-if="form.ocrMode === 'openai'" class="setting-card">
          <div class="setting-info">
            <label for="ocr-openai-api" class="setting-title">{{ OCR_MANAGE_TEXT.api }}</label>
            <p class="setting-description">{{ OCR_MANAGE_TEXT.apiDescription }}</p>
          </div>
          <div class="setting-control ocr-mode-control">
            <select
              id="ocr-openai-api"
              v-model="selectedAIConfig.api"
              :disabled="isSaving"
              class="select-input"
            >
              <option value="chat">{{ OCR_MANAGE_TEXT.chat }}</option>
              <option value="responses">{{ OCR_MANAGE_TEXT.responses }}</option>
            </select>
          </div>
        </div>
        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <label for="ocr-ai-base-url" class="setting-title">{{ AI_OCR_TEXT.baseUrl }}</label>
            <p class="setting-description">{{ AI_OCR_TEXT.baseUrlDescription }}</p>
            <input
              id="ocr-ai-base-url"
              v-model.trim="selectedAIConfig.baseUrl"
              :disabled="isSaving"
              class="text-input"
              type="url"
              spellcheck="false"
              :placeholder="
                createDefaultAIOCRConfigs()[form.ocrMode === 'claude' ? 'claude' : 'openai'].baseUrl
              "
            />
          </div>
        </div>
        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <label for="ocr-ai-key" class="setting-title">{{ AI_OCR_TEXT.apiKey }}</label>
            <p class="setting-description">{{ AI_OCR_TEXT.apiKeyDescription }}</p>
            <input
              id="ocr-ai-key"
              v-model.trim="selectedAIConfig.apiKey"
              :disabled="isSaving"
              class="text-input"
              type="password"
              autocomplete="off"
              spellcheck="false"
              :placeholder="AI_OCR_TEXT.apiKeyPlaceholder"
            />
          </div>
        </div>
        <div class="setting-card multi-line-card">
          <div class="setting-info full-width">
            <label for="ocr-ai-model" class="setting-title">{{ AI_OCR_TEXT.model }}</label>
            <p class="setting-description">{{ AI_OCR_TEXT.modelDescription }}</p>
            <div class="save-actions">
              <button
                class="shortcut-button secondary"
                :disabled="
                  isSaving ||
                  fetchingModels ||
                  !selectedAIConfig.apiKey ||
                  !selectedAIConfig.baseUrl
                "
                @click="fetchModels"
              >
                {{ fetchingModels ? OCR_MANAGE_TEXT.fetchingModels : OCR_MANAGE_TEXT.fetchModels }}
              </button>
            </div>
            <div ref="modelPicker" class="model-picker" @focusout="handleModelFocusOut">
              <input
                id="ocr-ai-model"
                ref="modelInput"
                v-model.trim="selectedAIConfig.model"
                :disabled="isSaving"
                class="text-input"
                type="text"
                role="combobox"
                aria-autocomplete="none"
                aria-haspopup="listbox"
                :aria-expanded="modelListOpen"
                :aria-controls="modelListOpen ? 'ocr-ai-model-options' : undefined"
                :aria-activedescendant="
                  modelListOpen && activeModelIndex >= 0
                    ? `ocr-ai-model-option-${activeModelIndex}`
                    : undefined
                "
                autocomplete="off"
                spellcheck="false"
                :placeholder="AI_OCR_TEXT.modelPlaceholder"
                @click="openModelList"
                @keydown="handleModelKeydown"
              />
              <button
                type="button"
                class="model-picker-toggle"
                :disabled="isSaving || !models.length"
                :aria-label="
                  modelListOpen ? OCR_MANAGE_TEXT.hideModels : OCR_MANAGE_TEXT.showModels
                "
                :aria-expanded="modelListOpen"
                :aria-controls="modelListOpen ? 'ocr-ai-model-options' : undefined"
                @click="toggleModelList"
              >
                <IconChevronDown aria-hidden="true" />
              </button>
              <div
                v-if="modelListOpen"
                id="ocr-ai-model-options"
                ref="modelList"
                class="model-options"
                role="listbox"
                :aria-label="AI_OCR_TEXT.model"
              >
                <button
                  v-for="(model, index) in models"
                  :id="`ocr-ai-model-option-${index}`"
                  :key="model.id"
                  type="button"
                  class="model-option"
                  :class="{ active: index === activeModelIndex }"
                  role="option"
                  :aria-selected="model.id === selectedAIConfig.model"
                  tabindex="-1"
                  @mousedown.prevent
                  @mouseenter="activeModelIndex = index"
                  @click="selectAIModel(model)"
                >
                  <span>{{ model.name || model.id }}</span>
                  <span v-if="model.name && model.name !== model.id" class="model-option-id">
                    {{ model.id }}
                  </span>
                </button>
              </div>
            </div>
            <p class="setting-description">{{ OCR_MANAGE_TEXT.testDescription }}</p>
            <div class="save-actions">
              <button
                class="shortcut-button secondary"
                :disabled="
                  isSaving ||
                  testingConnection ||
                  !selectedAIConfig.apiKey ||
                  !selectedAIConfig.baseUrl ||
                  !selectedAIConfig.model
                "
                @click="testConnection"
              >
                {{ testingConnection ? OCR_MANAGE_TEXT.testing : OCR_MANAGE_TEXT.test }}
              </button>
            </div>
            <p v-if="apiMessage" class="save-message" role="status">{{ apiMessage }}</p>
            <p v-if="apiError" class="field-error" role="alert">{{ apiError }}</p>
          </div>
        </div>
      </template>

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
              :disabled="!hasUnsavedChanges || isSaving || localBusy"
              @click="saveConfig"
            >
              {{ isSaving ? '正在保存…' : '保存 OCR 配置' }}
            </button>
            <button
              class="shortcut-button secondary"
              :disabled="!hasUnsavedChanges || isSaving || localBusy"
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
  min-width: 0;
  max-width: 100%;
  margin-top: 12px;
  padding: 5px 9px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  font-size: 13px;
  background: var(--color-bg-primary);
  transition:
    border-color var(--duration-fast),
    box-shadow var(--duration-fast);
  box-sizing: border-box;
}

.ocr-mode-control .select-input {
  margin-top: 0;
}

.model-picker {
  position: relative;
  margin-top: 12px;
}

.model-picker .text-input {
  margin-top: 0;
  padding-right: 36px;
}

.model-picker-toggle {
  position: absolute;
  top: 1px;
  right: 1px;
  bottom: 1px;
  display: grid;
  place-items: center;
  width: 30px;
  padding: 0;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--color-text-secondary);
  cursor: pointer;
}

.model-picker-toggle:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.model-picker-toggle:focus-visible {
  outline: 2px solid var(--color-primary);
  outline-offset: -2px;
}

.model-options {
  position: absolute;
  z-index: var(--z-dropdown);
  top: calc(100% + 4px);
  right: 0;
  left: 0;
  max-height: 240px;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: 4px;
  border: 1px solid var(--color-control-border);
  border-radius: var(--radius-xs);
  background: var(--color-bg-elevated);
  box-shadow: var(--shadow-elevated);
}

.model-option {
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
  padding: 7px 9px;
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--color-text-primary);
  font-size: 13px;
  text-align: left;
  overflow-wrap: anywhere;
  cursor: pointer;
}

.model-option:hover,
.model-option.active {
  background: var(--color-bg-secondary);
}

.model-option[aria-selected='true'] {
  color: var(--color-primary);
}

.model-option-id {
  color: var(--color-text-secondary);
  font-size: 12px;
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
  border-color: var(--color-primary);
  box-shadow: var(--shadow-focus-primary);
}

.field-error {
  margin-top: 8px;
  font-size: 12px;
  color: var(--color-danger);
}

.save-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 12px;
}

.shortcut-button:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.unsaved-text {
  margin-top: 10px;
  font-size: 12px;
  color: var(--color-warning-text);
}

.save-message {
  margin-top: 10px;
  font-size: 12px;
  color: var(--color-success-text);
}

.model-description,
.model-status {
  margin-top: 10px;
  font-size: 12px;
  color: var(--color-text-secondary);
}
.model-status.field-error {
  color: var(--color-danger);
}
.model-description,
.model-status,
.save-message,
.field-error {
  overflow-wrap: anywhere;
}

.download-progress {
  display: flex;
  flex-direction: column;
  gap: 8px;
  margin-top: 12px;
  font-size: 12px;
  color: var(--color-text-secondary);
}
.download-progress progress {
  width: 100%;
  height: 8px;
  accent-color: var(--color-primary);
}
</style>
