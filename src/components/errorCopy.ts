import type { AppErrorCode, UiLocale } from "../contracts/ipc";

/** Operator-facing copy shared by every surface that reports a failure. */
const englishErrorCopy: Record<AppErrorCode, { title: string; guidance: string }> = {
  "permission-denied": {
    title: "Accessibility Permission Required",
    guidance: "Allow selection access in System Settings.",
  },
  "unsupported-control": {
    title: "Text Control Not Supported",
    guidance: "Select text in a standard editable or document control.",
  },
  "no-selection": {
    title: "No Text Selected",
    guidance: "Select text, then try again.",
  },
  "missing-credential": {
    title: "API Key Required",
    guidance: "Save an API key in Settings.",
  },
  "invalid-credential": {
    title: "API Key Not Accepted",
    guidance: "Replace the API key in Settings.",
  },
  "api-restricted": {
    title: "API Access Restricted",
    guidance: "Allow translation access for this API key.",
  },
  "billing-required": {
    title: "Billing Required",
    guidance: "Enable billing for the translation service.",
  },
  "quota-exceeded": {
    title: "Translation Quota Reached",
    guidance: "Check your service quota, then retry.",
  },
  offline: {
    title: "You’re Offline",
    guidance: "Reconnect to the internet, then retry.",
  },
  timeout: {
    title: "Translation Timed Out",
    guidance: "Check your connection, then retry.",
  },
  "service-unavailable": {
    title: "Translation Service Unavailable",
    guidance: "Wait a moment, then retry.",
  },
  "invalid-language-pair": {
    title: "Language Pair Not Supported",
    guidance: "Choose a different source or target language.",
  },
  internal: {
    title: "Translation Failed",
    guidance: "Dismiss this result and try again.",
  },
};

const chineseErrorCopy: Record<AppErrorCode, { title: string; guidance: string }> = {
  "permission-denied": {
    title: "需要辅助功能权限",
    guidance: "请在系统设置中允许划词访问。",
  },
  "unsupported-control": {
    title: "暂不支持此文本控件",
    guidance: "请在标准编辑框或文档中选择文本。",
  },
  "no-selection": {
    title: "尚未选择文本",
    guidance: "请选择文本后重试。",
  },
  "missing-credential": {
    title: "需要 API 密钥",
    guidance: "请在设置中保存 API 密钥。",
  },
  "invalid-credential": {
    title: "API 密钥无效",
    guidance: "请在设置中更换 API 密钥。",
  },
  "api-restricted": {
    title: "API 访问受限",
    guidance: "请为此密钥开启翻译服务权限。",
  },
  "billing-required": {
    title: "需要启用计费",
    guidance: "请为所选翻译服务启用计费。",
  },
  "quota-exceeded": {
    title: "翻译额度已用尽",
    guidance: "请检查服务额度后重试。",
  },
  offline: {
    title: "当前处于离线状态",
    guidance: "请重新连接网络后重试。",
  },
  timeout: {
    title: "翻译请求超时",
    guidance: "请检查网络连接后重试。",
  },
  "service-unavailable": {
    title: "翻译服务暂不可用",
    guidance: "请稍后重试。",
  },
  "invalid-language-pair": {
    title: "不支持此语言组合",
    guidance: "请选择其他源语言或目标语言。",
  },
  internal: {
    title: "翻译失败",
    guidance: "请关闭此结果后重试。",
  },
};

export function errorCopy(locale: UiLocale) {
  return locale === "zh-CN" ? chineseErrorCopy : englishErrorCopy;
}
