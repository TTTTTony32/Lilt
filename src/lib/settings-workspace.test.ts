import { describe, expect, it } from "vitest";
import { DEFAULT_SNAPSHOT } from "../types/contracts";
import { createSettingsWorkspace, createWorkspacePrompt, createWorkspaceProvider, isSettingsWorkspaceDirty, workspaceSaveInput } from "./settings-workspace";

describe("settings workspace drafts", () => {
  it("detects edits and recognizes a reverted draft", () => {
    const original = createSettingsWorkspace(DEFAULT_SNAPSHOT);
    const edited = { ...original, settings: { ...original.settings, cacheEnabled: !original.settings.cacheEnabled } };
    expect(isSettingsWorkspaceDirty(edited, original)).toBe(true);
    expect(isSettingsWorkspaceDirty({ ...edited, settings: { ...edited.settings, cacheEnabled: original.settings.cacheEnabled } }, original)).toBe(false);
  });

  it("keeps provider and prompt additions and removals in the save payload", () => {
    const original = createSettingsWorkspace({
      ...DEFAULT_SNAPSHOT,
      prompts: [{ id: "builtin-general", name: "默认", content: "内容", sourceLanguage: "auto", targetLanguage: "zh-CN", version: 1, isBuiltin: true },
        { id: "old-prompt", name: "旧提示词", content: "", sourceLanguage: "auto", targetLanguage: "zh-CN", version: 1, isBuiltin: false }],
    });
    const provider = createWorkspaceProvider(original, "new-provider");
    const prompt = createWorkspacePrompt(original, "new-prompt");
    const draft = { ...original, activeProviderId: provider.id, providers: [...original.providers, { ...provider, apiKey: " secret " }], prompts: [original.prompts[0]!, prompt] };
    const input = workspaceSaveInput(draft, original);
    expect(isSettingsWorkspaceDirty(draft, original)).toBe(true);
    expect(input.activeProviderId).toBe("new-provider");
    expect(input.providers.find((item) => item.id === "new-provider")?.apiKey).toBe("secret");
    expect(input.deletedPromptIds).toEqual(["old-prompt"]);
    expect(input.prompts.map((item) => item.id)).toContain("new-prompt");
    expect(workspaceSaveInput(original, original).deletedProviderIds).toEqual([]);
  });
});
