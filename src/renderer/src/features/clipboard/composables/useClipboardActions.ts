import { clipboard } from '@renderer/api/clipboard'
import type { OCRTaskStatus } from '@shared/ai-ocr'
import { OCR_MANAGE_TEXT } from '@renderer/constants/ocr'
import { OverlayError, OverlayErrorCode } from '@renderer/types/overlay'

export const ensureValidItemId = (itemId: string): void => {
  if (!itemId || typeof itemId !== 'string') {
    throw new OverlayError('无效的剪贴板项目ID', OverlayErrorCode.INVALID_STATE, {
      itemId,
      type: typeof itemId
    })
  }
}

interface ClipboardAction {
  typeName: string
  action: string
  execute: (itemId: string) => Promise<boolean>
}

const actions = {
  restore: { typeName: '', action: '还原', execute: clipboard.restore },
  restoreAsText: { typeName: '纯文本', action: '还原', execute: clipboard.restoreAsText },
  restoreOcrText: { typeName: 'OCR文字', action: '还原', execute: clipboard.restoreOcrText },
  parseQRCode: { typeName: '二维码', action: '解析', execute: clipboard.parseQRCode },
  generateQRCode: {
    typeName: '二维码',
    action: '生成',
    execute: async (itemId: string): Promise<boolean> => {
      const result = await clipboard.generateQRCode(itemId)
      if (!result.success) {
        throw new OverlayError(
          `生成二维码失败: ${result.message}`,
          OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
          { itemId }
        )
      }
      return true
    }
  }
} satisfies Record<string, ClipboardAction>

interface ActionOptions {
  getTransitionId: () => number
  normalizeAfterAction: () => Promise<void>
  applyOCRStatus: (status: OCRTaskStatus) => void
}

type ClipboardActions = Record<
  | 'restoreClipboardContent'
  | 'restoreClipboardContentAsText'
  | 'restoreClipboardContentAsOcrText'
  | 'parseQRCodeFromClipboardItem'
  | 'generateQRCodeFromClipboardItem'
  | 'parseOcrFromClipboardItem',
  (itemId: string) => Promise<void>
>

export function useClipboardActions({
  getTransitionId,
  normalizeAfterAction,
  applyOCRStatus
}: ActionOptions): ClipboardActions {
  const restoreClipboard = async (itemId: string, method: keyof typeof actions): Promise<void> => {
    const transitionId = getTransitionId()
    const { execute, typeName, action } = actions[method]
    try {
      ensureValidItemId(itemId)
      if (!(await execute(itemId))) {
        throw new OverlayError(
          `剪贴板${typeName}${action}操作失败`,
          OverlayErrorCode.CLIPBOARD_LOAD_FAILED,
          { itemId }
        )
      }
      if (transitionId === getTransitionId()) await normalizeAfterAction()
    } catch (error) {
      console.error(`[前端] ${action}剪贴板${typeName ? ` ${typeName}` : ''}失败:`, error)
    }
  }
  const parseOcrFromClipboardItem = async (itemId: string): Promise<void> => {
    try {
      ensureValidItemId(itemId)

      await clipboard.parseOcr(itemId)
    } catch (error) {
      applyOCRStatus({
        contentId: itemId,
        state: 'error',
        error:
          error instanceof Error
            ? error.message
            : typeof error === 'string'
              ? error
              : OCR_MANAGE_TEXT.error
      })
      console.error('[前端] 解析OCR失败:', error)
    }
  }

  return {
    restoreClipboardContent: (id: string) => restoreClipboard(id, 'restore'),
    restoreClipboardContentAsText: (id: string) => restoreClipboard(id, 'restoreAsText'),
    restoreClipboardContentAsOcrText: (id: string) => restoreClipboard(id, 'restoreOcrText'),
    parseQRCodeFromClipboardItem: (id: string) => restoreClipboard(id, 'parseQRCode'),
    generateQRCodeFromClipboardItem: (id: string) => restoreClipboard(id, 'generateQRCode'),
    parseOcrFromClipboardItem
  }
}
