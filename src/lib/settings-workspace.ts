import type { AppSettings, AppSnapshot, ModelInfo, Prompt, ProviderConfig } from "../types/contracts";

export interface WorkspaceProvider extends ProviderConfig {
  apiKey: string;
  fetchedModels: ModelInfo[] | null;
}

export interface SettingsWorkspaceDraft {
  activeProviderId: string;
  providers: WorkspaceProvider[];
  prompts: Prompt[];
  settings: AppSettings;
}

export function createSettingsWorkspace(snapshot: AppSnapshot): SettingsWorkspaceDraft {
  return {
    activeProviderId: snapshot.provider.id,
    providers: snapshot.providers.map((provider) => ({ ...provider, apiKey: "", fetchedModels: null })),
    prompts: snapshot.prompts.map((prompt) => ({ ...prompt })),
    settings: { ...snapshot.settings },
  };
}

function comparableWorkspace(draft: SettingsWorkspaceDraft) {
  return {
    activeProviderId: draft.activeProviderId,
    providers: draft.providers.map(({ fetchedModels: _models, hasApiKey: _hasApiKey, ...provider }) => provider),
    prompts: draft.prompts,
    settings: {
      historyRetention: draft.settings.historyRetention,
      cacheEnabled: draft.settings.cacheEnabled,
      cacheMaxBytes: draft.settings.cacheMaxBytes,
      wordAiCacheEnabled: draft.settings.wordAiCacheEnabled,
      paragraphExampleLookupEnabled: draft.settings.paragraphExampleLookupEnabled,
      pdfPreflightPageLimit: draft.settings.pdfPreflightPageLimit,
      pdfNetworkConcurrency: draft.settings.pdfNetworkConcurrency,
      selectionMode: draft.settings.selectionMode,
      selectionShortcut: draft.settings.selectionShortcut,
      closeBehavior: draft.settings.closeBehavior,
    },
  };
}

export function isSettingsWorkspaceDirty(draft: SettingsWorkspaceDraft, original: SettingsWorkspaceDraft): boolean {
  return JSON.stringify(comparableWorkspace(draft)) !== JSON.stringify(comparableWorkspace(original));
}

export function createWorkspaceProvider(draft: SettingsWorkspaceDraft, id: string): WorkspaceProvider {
  const names = new Set(draft.providers.map((provider) => provider.name));
  let number = 1;
  while (names.has(`自定义供应商${number}`)) number += 1;
  return {
    id,
    name: `自定义供应商${number}`,
    baseUrl: "https://api.openai.com/v1",
    modelId: "gpt-4o-mini",
    promptId: draft.prompts.find((prompt) => prompt.id === "builtin-general")?.id ?? draft.prompts[0]?.id ?? "builtin-general",
    thinkingEffort: "none",
    apiProtocol: "chat_completions",
    hasApiKey: false,
    isBuiltin: false,
    apiKey: "",
    fetchedModels: null,
  };
}

export function createWorkspacePrompt(draft: SettingsWorkspaceDraft, id: string): Prompt {
  const names = new Set(draft.prompts.map((prompt) => prompt.name));
  let number = 1;
  while (names.has(`自定义提示词${number}`)) number += 1;
  return { id, name: `自定义提示词${number}`, content: "", sourceLanguage: "auto", targetLanguage: "zh-CN", version: 1, isBuiltin: false };
}

export function workspaceSaveInput(draft: SettingsWorkspaceDraft, original: SettingsWorkspaceDraft) {
  const providerIds = new Set(draft.providers.map((provider) => provider.id));
  const promptIds = new Set(draft.prompts.map((prompt) => prompt.id));
  return {
    activeProviderId: draft.activeProviderId,
    providers: draft.providers.map(({ hasApiKey: _hasApiKey, isBuiltin: _isBuiltin, fetchedModels, apiKey, ...provider }) => ({
      ...provider,
      apiKey: apiKey.trim() || null,
      models: fetchedModels,
    })),
    deletedProviderIds: original.providers.filter((provider) => !providerIds.has(provider.id)).map((provider) => provider.id),
    prompts: draft.prompts.map(({ version: _version, isBuiltin: _isBuiltin, ...prompt }) => prompt),
    deletedPromptIds: original.prompts.filter((prompt) => !promptIds.has(prompt.id)).map((prompt) => prompt.id),
    settings: {
      historyRetention: draft.settings.historyRetention,
      cacheEnabled: draft.settings.cacheEnabled,
      cacheMaxBytes: draft.settings.cacheMaxBytes,
      wordAiCacheEnabled: draft.settings.wordAiCacheEnabled,
      paragraphExampleLookupEnabled: draft.settings.paragraphExampleLookupEnabled,
      pdfPreflightPageLimit: draft.settings.pdfPreflightPageLimit,
      pdfNetworkConcurrency: draft.settings.pdfNetworkConcurrency,
      selectionMode: draft.settings.selectionMode,
      selectionShortcut: draft.settings.selectionShortcut,
      closeBehavior: draft.settings.closeBehavior,
    },
  };
}
