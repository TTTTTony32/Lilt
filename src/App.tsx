import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Copy, FileText, History, Languages, LoaderCircle, Settings, Square, WandSparkles } from "lucide-react";
import { describeError } from "./lib/errors";
import { invokeCommand, listenTo } from "./lib/tauri";
import {
  DEFAULT_SNAPSHOT,
  type AppSettings,
  type AppSnapshot,
  type AppTab,
  type GlossaryTerm,
  type HistoryEntry,
  type ModelInfo,
  type TranslationEvent,
  type TranslationStatus,
  decodeTranslationEvent,
} from "./types/contracts";

const EVENT_NAMES = [
  "translation_started",
  "translation_delta",
  "translation_completed",
  "translation_cancelled",
  "translation_failed",
] as const;

const LANGUAGE_OPTIONS = [
  ["英语", "en"],
  ["简体中文", "zh-CN"],
  ["繁体中文", "zh-TW"],
  ["日语", "ja"],
  ["韩语", "ko"],
] as const;

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDate(value: string): string {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(value));
}

function App() {
  const [snapshot, setSnapshot] = useState<AppSnapshot>(DEFAULT_SNAPSHOT);
  const [tab, setTab] = useState<AppTab>("translate");
  const [sourceText, setSourceText] = useState("");
  const [translatedText, setTranslatedText] = useState("");
  const [sourceLanguage, setSourceLanguage] = useState("en");
  const [targetLanguage, setTargetLanguage] = useState("zh-CN");
  const [status, setStatus] = useState<TranslationStatus>("idle");
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [lastCacheHit, setLastCacheHit] = useState(false);
  const activeRequestId = useRef<string | null>(null);

  const refreshSnapshot = useCallback(async () => {
    try {
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      setSnapshot(next);
    } catch (reason) {
      setError(describeError(reason, "无法读取应用配置"));
    }
  }, []);

  useEffect(() => {
    void refreshSnapshot();
  }, [refreshSnapshot]);

  const handleEvent = useCallback((event: TranslationEvent) => {
    if (event.requestId !== activeRequestId.current) return;
    switch (event.type) {
      case "started":
        setStatus("streaming");
        break;
      case "delta":
        setTranslatedText((current) => current + event.content);
        break;
      case "completed":
        setTranslatedText(event.content);
        setLastCacheHit(event.cacheHit);
        setStatus("completed");
        void refreshSnapshot();
        break;
      case "cancelled":
        setStatus("idle");
        break;
      case "failed":
        setError(event.message);
        setStatus("failed");
        break;
    }
  }, [refreshSnapshot]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    void Promise.all(EVENT_NAMES.map(async (name) => {
      const unlisten = await listenTo<unknown>(name, (payload) => {
        if (disposed) return;
        const event = decodeTranslationEvent(name, payload);
        if (event) handleEvent(event);
      });
      if (disposed) unlisten();
      else unlisteners.push(unlisten);
    }));
    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, [handleEvent]);

  const selectedModel = useMemo(() => {
    const known = snapshot.models.find((model) => model.id === snapshot.provider.modelId);
    return known?.label ?? snapshot.provider.modelId;
  }, [snapshot.models, snapshot.provider.modelId]);

  const handleTranslate = async () => {
    const text = sourceText.trim();
    if (!text) {
      setError("请先输入需要翻译的段落。");
      return;
    }
    const requestId = crypto.randomUUID();
    activeRequestId.current = requestId;
    setError(null);
    setNotice(null);
    setTranslatedText("");
    setLastCacheHit(false);
    setStatus("streaming");
    try {
      await invokeCommand("translate", {
        request: {
          requestId,
          sourceText: text,
          sourceLanguage,
          targetLanguage,
          modelId: snapshot.provider.modelId,
          promptId: snapshot.provider.promptId,
        },
      });
    } catch (reason) {
      setError(describeError(reason, "翻译请求失败"));
      setStatus("failed");
    }
  };

  const handleCancel = async () => {
    const requestId = activeRequestId.current;
    if (!requestId) return;
    setStatus("cancelling");
    try {
      await invokeCommand("cancel_translation", { requestId });
    } catch (reason) {
      setError(describeError(reason, "取消请求失败"));
    }
  };

  const handleCopy = async () => {
    if (!translatedText) return;
    await navigator.clipboard.writeText(translatedText);
    setNotice("译文已复制");
    window.setTimeout(() => setNotice(null), 1800);
  };

  const handleSettingsSaved = (next: AppSnapshot) => {
    setSnapshot(next);
    setNotice("设置已保存");
    window.setTimeout(() => setNotice(null), 1800);
  };

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand-lockup">
          <div className="brand-mark">L</div>
          <div>
            <div className="brand-name">Lilt</div>
            <div className="brand-caption">阅读辅助工具</div>
          </div>
        </div>
        <div className="topbar-status">
          <span className="status-dot" />
          本地运行
        </div>
      </header>

      <div className="workspace">
        <aside className="sidebar">
          <nav className="nav-list" aria-label="主导航">
            <NavItem icon={<Languages size={17} />} label="段落翻译" active={tab === "translate"} onClick={() => setTab("translate")} />
            <NavItem icon={<FileText size={17} />} label="术语表" active={tab === "glossary"} onClick={() => setTab("glossary")} />
            <NavItem icon={<History size={17} />} label="翻译历史" active={tab === "history"} onClick={() => setTab("history")} />
            <NavItem icon={<Settings size={17} />} label="设置" active={tab === "settings"} onClick={() => setTab("settings")} />
          </nav>
          <div className="sidebar-note">
            <WandSparkles size={16} />
            <span>首版聚焦段落翻译</span>
          </div>
        </aside>

        <main className="main-content">
          {tab === "translate" && (
            <TranslateView
              sourceText={sourceText}
              translatedText={translatedText}
              sourceLanguage={sourceLanguage}
              targetLanguage={targetLanguage}
              selectedModel={selectedModel}
              status={status}
              error={error}
              notice={notice}
              cacheHit={lastCacheHit}
              onSourceTextChange={setSourceText}
              onSourceLanguageChange={setSourceLanguage}
              onTargetLanguageChange={setTargetLanguage}
              onTranslate={() => void handleTranslate()}
              onCancel={() => void handleCancel()}
              onCopy={() => void handleCopy()}
            />
          )}
          {tab === "glossary" && (
            <GlossaryView terms={snapshot.glossaryTerms} onChanged={() => void refreshSnapshot()} />
          )}
          {tab === "history" && <HistoryView history={snapshot.history} />}
          {tab === "settings" && <SettingsView snapshot={snapshot} onSaved={handleSettingsSaved} />}
        </main>
      </div>
    </div>
  );
}

function NavItem({ icon, label, active, onClick }: { icon: React.ReactNode; label: string; active: boolean; onClick: () => void }) {
  return (
    <button className={`nav-item ${active ? "is-active" : ""}`} onClick={onClick} type="button">
      {icon}
      <span>{label}</span>
    </button>
  );
}

interface TranslateViewProps {
  sourceText: string;
  translatedText: string;
  sourceLanguage: string;
  targetLanguage: string;
  selectedModel: string;
  status: TranslationStatus;
  error: string | null;
  notice: string | null;
  cacheHit: boolean;
  onSourceTextChange: (value: string) => void;
  onSourceLanguageChange: (value: string) => void;
  onTargetLanguageChange: (value: string) => void;
  onTranslate: () => void;
  onCancel: () => void;
  onCopy: () => void;
}

function TranslateView(props: TranslateViewProps) {
  const isBusy = props.status === "streaming" || props.status === "cancelling";
  return (
    <section className="page-section translate-page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">TRANSLATE</p>
          <h1>段落翻译</h1>
          <p className="page-description">保留上下文，把一段文字译成适合阅读的中文。</p>
        </div>
        <div className="model-pill">{props.selectedModel || "未配置模型"}</div>
      </div>

      <div className="translation-grid">
        <div className="translation-panel">
          <div className="panel-toolbar">
            <label htmlFor="source-language">原文</label>
            <select id="source-language" value={props.sourceLanguage} onChange={(event) => props.onSourceLanguageChange(event.target.value)}>
              {LANGUAGE_OPTIONS.map(([label, value]) => <option key={value} value={value}>{label}</option>)}
            </select>
          </div>
          <textarea
            value={props.sourceText}
            onChange={(event) => props.onSourceTextChange(event.target.value)}
            placeholder="粘贴需要翻译的英文段落……"
            spellCheck={false}
          />
          <div className="panel-footer"><span>{props.sourceText.length} 字符</span></div>
        </div>

        <div className="translation-panel result-panel">
          <div className="panel-toolbar">
            <label htmlFor="target-language">译文</label>
            <select id="target-language" value={props.targetLanguage} onChange={(event) => props.onTargetLanguageChange(event.target.value)}>
              {LANGUAGE_OPTIONS.map(([label, value]) => <option key={value} value={value}>{label}</option>)}
            </select>
          </div>
          <div className={`result-content ${props.translatedText ? "has-content" : ""}`}>
            {props.translatedText || <span className="empty-result">译文会显示在这里</span>}
            {props.status === "streaming" && <span className="stream-caret" />}
          </div>
          <div className="panel-footer result-footer">
            <span>{props.cacheHit ? "来自段落缓存" : props.status === "completed" ? "已完成" : ""}</span>
            <button className="icon-button" title="复制译文" aria-label="复制译文" onClick={props.onCopy} disabled={!props.translatedText} type="button"><Copy size={16} /></button>
          </div>
        </div>
      </div>

      <div className="action-row">
        <div className="message-area">
          {props.error && <span className="error-message">{props.error}</span>}
          {!props.error && props.notice && <span className="notice-message">{props.notice}</span>}
        </div>
        {isBusy ? (
          <button className="primary-button cancel-button" type="button" onClick={props.onCancel} disabled={props.status === "cancelling"}>
            {props.status === "cancelling" ? <LoaderCircle className="spin" size={16} /> : <Square size={14} fill="currentColor" />}
            {props.status === "cancelling" ? "正在取消" : "取消翻译"}
          </button>
        ) : (
          <button className="primary-button" type="button" onClick={props.onTranslate}>
            <WandSparkles size={16} />
            开始翻译
          </button>
        )}
      </div>

      <div className="quick-facts">
        <div><span className="fact-label">当前方向</span><strong>{languageName(props.sourceLanguage)} → {languageName(props.targetLanguage)}</strong></div>
        <div><span className="fact-label">段落缓存</span><strong>默认开启</strong></div>
        <div><span className="fact-label">历史记录</span><strong>始终保留</strong></div>
      </div>
    </section>
  );
}

function languageName(value: string): string {
  return LANGUAGE_OPTIONS.find(([, code]) => code === value)?.[0] ?? value;
}

function GlossaryView({ terms, onChanged }: { terms: GlossaryTerm[]; onChanged: () => void }) {
  const [source, setSource] = useState("");
  const [target, setTarget] = useState("");
  const [note, setNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const addTerm = async () => {
    if (!source.trim() || !target.trim()) {
      setError("原文术语和译文都不能为空。");
      return;
    }
    try {
      await invokeCommand("upsert_glossary_term", { source: source.trim(), target: target.trim(), note: note.trim() || null });
      setSource(""); setTarget(""); setNote(""); setError(null); onChanged();
    } catch (reason) {
      setError(describeError(reason, "术语保存失败"));
    }
  };
  return (
    <section className="page-section narrow-page">
      <PageTitle eyebrow="GLOSSARY" title="术语表" description="全局术语会在段落翻译时按命中情况加入提示。" />
      <div className="simple-card">
        <div className="form-grid glossary-form">
          <label>原文<input value={source} onChange={(event) => setSource(event.target.value)} placeholder="例如：embedding" /></label>
          <label>译文<input value={target} onChange={(event) => setTarget(event.target.value)} placeholder="例如：嵌入" /></label>
          <label className="wide-field">备注<input value={note} onChange={(event) => setNote(event.target.value)} placeholder="可选" /></label>
        </div>
        <div className="form-actions"><span className="error-message">{error}</span><button className="secondary-button" type="button" onClick={() => void addTerm()}>添加术语</button></div>
      </div>
      <div className="list-card">
        <div className="list-card-heading"><strong>已添加术语</strong><span>{terms.length} 条</span></div>
        {terms.length === 0 ? <div className="empty-list">还没有术语。</div> : terms.map((term) => <GlossaryRow key={term.id} term={term} onChanged={onChanged} />)}
      </div>
    </section>
  );
}

function GlossaryRow({ term, onChanged }: { term: GlossaryTerm; onChanged: () => void }) {
  const remove = async () => {
    await invokeCommand("delete_glossary_term", { id: term.id });
    onChanged();
  };
  return <div className="list-row"><div><strong>{term.source}</strong><span className="arrow">→</span><span>{term.target}</span>{term.note && <small>{term.note}</small>}</div><button className="text-button danger-text" type="button" onClick={() => void remove()}>删除</button></div>;
}

function HistoryView({ history }: { history: HistoryEntry[] }) {
  return (
    <section className="page-section narrow-page">
      <PageTitle eyebrow="HISTORY" title="翻译历史" description="历史记录属于客户端固定功能，按设置中的条数保留。" />
      <div className="list-card history-card">
        <div className="list-card-heading"><strong>最近翻译</strong><span>{history.length} 条</span></div>
        {history.length === 0 ? <div className="empty-list">完成一次段落翻译后，记录会出现在这里。</div> : history.map((item) => <HistoryRow key={item.id} item={item} />)}
      </div>
    </section>
  );
}

function HistoryRow({ item }: { item: HistoryEntry }) {
  return <article className="history-row"><div className="history-meta"><span>{formatDate(item.createdAt)}</span><span>{item.modelId}</span>{item.cacheHit && <span className="tag">缓存命中</span>}</div><p className="history-source">{item.sourceText}</p><p className="history-result">{item.translatedText}</p></article>;
}

function SettingsView({ snapshot, onSaved }: { snapshot: AppSnapshot; onSaved: (snapshot: AppSnapshot) => void }) {
  const [baseUrl, setBaseUrl] = useState(snapshot.provider.baseUrl);
  const [modelId, setModelId] = useState(snapshot.provider.modelId);
  const [promptId, setPromptId] = useState(snapshot.provider.promptId);
  const [apiKey, setApiKey] = useState("");
  const [settings, setSettings] = useState<AppSettings>(snapshot.settings);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setBaseUrl(snapshot.provider.baseUrl);
    setModelId(snapshot.provider.modelId);
    setPromptId(snapshot.provider.promptId);
    setSettings(snapshot.settings);
  }, [snapshot]);

  const saveProvider = async () => {
    setError(null);
    setMessage(null);
    try {
      await invokeCommand("save_provider_config", { baseUrl, modelId, promptId, apiKey: apiKey || null });
      setApiKey("");
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      onSaved(next);
      setMessage("Provider 设置已保存");
    } catch (reason) {
      setError(describeError(reason, "Provider 设置保存失败"));
      setMessage(null);
    }
  };

  const saveAppSettings = async () => {
    setError(null);
    setMessage(null);
    try {
      await invokeCommand("save_app_settings", { historyRetention: settings.historyRetention, cacheEnabled: settings.cacheEnabled, cacheMaxBytes: settings.cacheMaxBytes });
      const next = await invokeCommand<AppSnapshot>("get_app_snapshot");
      onSaved(next);
      setMessage("本地设置已保存");
    } catch (reason) {
      setError(describeError(reason, "本地设置保存失败"));
      setMessage(null);
    }
  };

  const fetchModels = async () => {
    setError(null);
    setMessage(null);
    try {
      const models = await invokeCommand<ModelInfo[]>("fetch_models", {
        baseUrl: baseUrl.trim() || null,
        apiKey: apiKey.trim() || null,
      });
      setMessage(`模型列表已更新，共 ${models.length} 个模型`);
    } catch (reason) {
      setError(describeError(reason, "模型列表读取失败，可手动填写 Model ID"));
      setMessage(null);
    }
  };

  return (
    <section className="page-section narrow-page">
      <PageTitle eyebrow="SETTINGS" title="设置" description="配置模型连接，并管理本地历史与段落缓存。" />
      <div className="settings-stack">
        <div className="simple-card">
          <div className="card-heading"><div><strong>OpenAI-compatible Provider</strong><span>首版仅支持这一种协议</span></div><span className={`connection-status ${snapshot.provider.hasApiKey ? "connected" : ""}`}>{snapshot.provider.hasApiKey ? "已配置密钥" : "未配置密钥"}</span></div>
          <div className="form-grid">
            <label className="wide-field">Base URL<input value={baseUrl} onChange={(event) => setBaseUrl(event.target.value)} placeholder="https://api.openai.com/v1" /></label>
            <label>Model ID<input value={modelId} onChange={(event) => setModelId(event.target.value)} placeholder="gpt-4o-mini" /></label>
            <label>Prompt<select value={promptId} onChange={(event) => setPromptId(event.target.value)}>{snapshot.prompts.map((prompt) => <option key={prompt.id} value={prompt.id}>{prompt.name} · v{prompt.version}</option>)}</select></label>
            <label className="wide-field">API Key<input type="password" value={apiKey} onChange={(event) => setApiKey(event.target.value)} placeholder={snapshot.provider.hasApiKey ? "已保存，留空表示不修改" : "保存在 Windows 凭据管理器"} autoComplete="off" /></label>
          </div>
          <div className="form-actions"><span className="muted-text">模型列表读取失败时，Model ID 仍可手动填写。</span><div className="button-group"><button className="secondary-button" type="button" onClick={() => void fetchModels()}>读取模型</button><button className="primary-button small-button" type="button" onClick={() => void saveProvider()}>保存 Provider</button></div></div>
        </div>

        <div className="simple-card">
          <div className="card-heading"><div><strong>本地数据</strong><span>数据只保存在当前设备</span></div></div>
          <label className="setting-line"><span><strong>翻译历史保留条数</strong><small>历史功能不可关闭，只控制保留数量。</small></span><input className="number-input" type="number" min={1} max={1000} value={settings.historyRetention} onChange={(event) => setSettings({ ...settings, historyRetention: Number(event.target.value) })} /></label>
          <label className="setting-line"><span><strong>启用段落翻译缓存</strong><small>缓存命中后仍会写入一条历史记录。</small></span><input type="checkbox" checked={settings.cacheEnabled} onChange={(event) => setSettings({ ...settings, cacheEnabled: event.target.checked })} /></label>
          <label className="setting-line slider-line"><span><strong>段落缓存上限</strong><small>已使用 {formatBytes(snapshot.cacheStats.usageBytes)}，上限 {formatBytes(settings.cacheMaxBytes)}。</small></span><input type="range" min={16} max={2048} step={16} value={Math.round(settings.cacheMaxBytes / (1024 * 1024))} onChange={(event) => setSettings({ ...settings, cacheMaxBytes: Number(event.target.value) * 1024 * 1024 })} /></label>
          <div className="form-actions"><span className="muted-text">缓存不包含 API Key。</span><button className="primary-button small-button" type="button" onClick={() => void saveAppSettings()}>保存本地设置</button></div>
        </div>
      </div>
      {message && <p className="notice-message settings-message">{message}</p>}
      {error && <p className="error-message settings-message">{error}</p>}
    </section>
  );
}

function PageTitle({ eyebrow, title, description }: { eyebrow: string; title: string; description: string }) {
  return <div className="page-heading"><div><p className="eyebrow">{eyebrow}</p><h1>{title}</h1><p className="page-description">{description}</p></div></div>;
}

export default App;
