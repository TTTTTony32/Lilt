import { useCallback, useEffect, useRef, useState } from "react";
import { Bookmark, Download, LoaderCircle, Search } from "lucide-react";
import { describeError } from "./lib/errors";
import { invokeCommand } from "./lib/tauri";
import {
  decodeDictionaryLookupCommandResult,
  collectPronunciations,
  groupRelationsByType,
  posLabelZh,
  splitMeaningsByPriority,
  type DictionaryEntry,
  type DictionaryHistoryEntry,
  type DictionaryLookupResult,
  type DictionaryMeaning,
  type DictionaryPosGroup,
  type DictionaryState,
  type DictionaryLookupCandidate,
  type ParagraphExample,
} from "./types/dictionary";
import type { PersonalDictionaryEntry, WordExampleState } from "./types/contracts";

export interface DictionaryProgress {
  operationId: string;
  phase: "download" | "verify" | "extract";
  current: number;
  total: number;
}

interface DictionaryViewProps {
  state: DictionaryState;
  history: DictionaryHistoryEntry[];
  progress: DictionaryProgress | null;
  targetLanguage: string;
  wordExample: WordExampleState;
  onUpdate: () => Promise<void>;
  onHistoryChanged: (history: DictionaryHistoryEntry[]) => void;
  onSnapshotChanged: () => Promise<void>;
  onWordExampleRequested: (request: WordExampleRequestInput | null) => void;
  onWordExampleCancelled: () => void;
  personalDictionary: PersonalDictionaryEntry[];
  openRequest: DictionaryOpenRequest | null;
  onOpenRequestHandled: () => void;
  onPersonalDictionaryChanged: () => Promise<void>;
}

export interface WordExampleRequestInput {
  exampleId: number;
  word: string;
  canonicalWord: string;
  targetLanguage: string;
}

export interface DictionaryOpenRequest {
  requestId: string;
  lookupWord: string;
  canonicalWord: string;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

function progressPercent(progress: DictionaryProgress | null): number {
  if (!progress || progress.total <= 0) return 0;
  return Math.min(100, Math.round((progress.current / progress.total) * 100));
}

function progressLabel(progress: DictionaryProgress | null): string {
  if (!progress) return "正在准备词典更新";
  if (progress.phase === "download") return `正在下载 ${progressPercent(progress)}%`;
  if (progress.phase === "verify") return `正在校验 ${progressPercent(progress)}%`;
  return `正在解压 ${progressPercent(progress)}%`;
}

export default function DictionaryView({
  state,
  history,
  progress,
  targetLanguage,
  wordExample,
  onUpdate,
  onHistoryChanged,
  onSnapshotChanged,
  onWordExampleRequested,
  onWordExampleCancelled,
  personalDictionary,
  openRequest,
  onOpenRequestHandled,
  onPersonalDictionaryChanged,
}: DictionaryViewProps) {
  const [word, setWord] = useState("");
  const [result, setResult] = useState<DictionaryEntry | null>(null);
  const [lookupMeta, setLookupMeta] = useState<DictionaryLookupResult | null>(null);
  const [example, setExample] = useState<ParagraphExample | null>(null);
  const [candidates, setCandidates] = useState<DictionaryLookupCandidate[]>([]);
  const [notFound, setNotFound] = useState(false);
  const [querying, setQuerying] = useState(false);
  const [queryError, setQueryError] = useState<string | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [savingFavorite, setSavingFavorite] = useState(false);
  const lastOpenRequestId = useRef<string | null>(null);
  const updating = state.status === "updating" || progress !== null;

  const query = useCallback(async (candidate: string, selectedCanonicalWord?: string) => {
    const trimmed = candidate.trim();
    if (!trimmed) {
      setQueryError("请输入要查询的词形。");
      return;
    }
    if (state.status !== "ready") {
      setQueryError(state.error ?? "词典尚未安装，请先下载词典。");
      return;
    }
    setQuerying(true);
    setQueryError(null);
    setNotFound(false);
    setCandidates([]);
    setResult(null);
    setLookupMeta(null);
    setExample(null);
    onWordExampleRequested(null);
    try {
      const rawResult = await invokeCommand<unknown>("query_dictionary", {
        word: trimmed,
        canonicalWord: selectedCanonicalWord ?? null,
      });
      const decoded = decodeDictionaryLookupCommandResult(rawResult);
      if (!decoded) {
        setQueryError("词典命令返回了无法识别的结果。");
        return;
      }
      setWord(decoded.lookup?.word ?? trimmed);
      setCandidates(decoded.candidates);
      setNotFound(decoded.lookup === null && decoded.candidates.length === 0);
      setResult(decoded.lookup?.entry ?? null);
      setLookupMeta(decoded.lookup);
      setExample(decoded.example);
      if (decoded.lookup && decoded.example) {
        onWordExampleRequested({
          exampleId: decoded.example.exampleId,
          word: decoded.lookup.word,
          canonicalWord: decoded.lookup.canonicalWord,
          targetLanguage,
        });
      }
      onHistoryChanged(decoded.history);
      setHistoryOpen(false);
    } catch (reason) {
      setResult(null);
      setLookupMeta(null);
      setExample(null);
      setCandidates([]);
      setNotFound(false);
      onWordExampleRequested(null);
      setQueryError(describeError(reason, "词典查询失败"));
    } finally {
      setQuerying(false);
    }
  }, [onHistoryChanged, onWordExampleRequested, state.error, state.status, targetLanguage]);

  useEffect(() => {
    if (!openRequest || openRequest.requestId === lastOpenRequestId.current) return;
    lastOpenRequestId.current = openRequest.requestId;
    setWord(openRequest.lookupWord);
    void query(openRequest.lookupWord, openRequest.canonicalWord).finally(onOpenRequestHandled);
  }, [onOpenRequestHandled, openRequest, query]);

  const isFavorite = lookupMeta !== null && personalDictionary.some(
    (item) => item.normalizedCanonicalWord === lookupMeta.canonicalWord.trim().toLowerCase(),
  );

  const toggleFavorite = async () => {
    if (!lookupMeta || savingFavorite) return;
    setSavingFavorite(true);
    try {
      if (isFavorite) {
        await invokeCommand("remove_personal_word", { canonicalWord: lookupMeta.canonicalWord });
      } else {
        await invokeCommand("save_personal_word", {
          lookupWord: lookupMeta.word,
          canonicalWord: lookupMeta.canonicalWord,
        });
      }
      await onPersonalDictionaryChanged();
    } catch (reason) {
      setQueryError(describeError(reason, isFavorite ? "移除个人词典词条失败" : "收藏词条失败"));
    } finally {
      setSavingFavorite(false);
    }
  };

  return (
    <section className="page-section dictionary-page">
      <div className="page-heading">
        <div>
          <p className="eyebrow">DICTIONARY</p>
          <h1>词典</h1>
          <p className="page-description">离线查询英语词条、中文释义和官方双语例句。</p>
        </div>
        {state.status === "ready" && state.installedRelease && (
          <div className="model-pill">{state.installedRelease}</div>
        )}
      </div>

      <div className="dictionary-search-card">
        <form className="dictionary-search-form" onSubmit={(event) => { event.preventDefault(); void query(word); }}>
          <div className="dictionary-input-wrap">
            <Search size={17} aria-hidden="true" />
            <input
              value={word}
              onChange={(event) => setWord(event.target.value)}
              onFocus={() => setHistoryOpen(true)}
              placeholder="输入英语词形，例如 resolve"
              aria-label="词典查询"
              autoComplete="off"
              disabled={updating}
            />
            {historyOpen && history.length > 0 && (
              <div className="dictionary-history-menu" role="listbox">
                {history.map((item) => (
                  <button
                    key={item.normalizedWord}
                    type="button"
                    role="option"
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => { setWord(item.displayWord); void query(item.displayWord); }}
                  >
                    <span>{item.displayWord}</span>
                    {item.queryCount > 1 && <small>{item.queryCount} 次</small>}
                  </button>
                ))}
                <button
                  type="button"
                  className="dictionary-history-clear"
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={async () => {
                    try {
                      await invokeCommand("clear_dictionary_history");
                      await onSnapshotChanged();
                      setHistoryOpen(false);
                    } catch (reason) {
                      setQueryError(describeError(reason, "清空词典历史失败"));
                    }
                  }}
                >
                  清空历史
                </button>
              </div>
            )}
          </div>
          <button className="primary-button" type="submit" disabled={querying || updating || state.status !== "ready"}>
            {querying ? <LoaderCircle className="spin" size={16} /> : <Search size={16} />}
            查询
          </button>
        </form>
      </div>

      {state.status !== "ready" && (
        <DictionaryInstallCard
          state={state}
          progress={progress}
          onUpdate={onUpdate}
        />
      )}

      {queryError && <p className="error-message dictionary-message">{queryError}</p>}
      {candidates.length > 0 && (
        <div className="dictionary-candidate-card">
          <strong>这个词形对应多个词头，请选择</strong>
          <div>
            {candidates.map((candidate) => (
              <button
                className="dictionary-candidate-button"
                key={candidate.normalizedCanonicalWord}
                type="button"
                onClick={() => void query(word, candidate.canonicalWord)}
              >
                {candidate.canonicalWord}
              </button>
            ))}
          </div>
        </div>
      )}
      {state.status === "ready" && notFound && !queryError && (
        <div className="dictionary-empty-state">没有找到对应词条。</div>
      )}
      {state.status === "ready" && !result && !notFound && candidates.length === 0 && !queryError && (
        <div className="dictionary-empty-state">输入词形后，结果会显示在这里。</div>
      )}
      {state.status === "ready" && result && (
        <DictionaryEntryView
          entry={result}
          lookup={lookupMeta}
          example={example}
          wordExample={wordExample}
          onCancelExample={onWordExampleCancelled}
          isFavorite={isFavorite}
          savingFavorite={savingFavorite}
          onFavoriteToggle={() => void toggleFavorite()}
        />
      )}
    </section>
  );
}

function DictionaryInstallCard({
  state,
  progress,
  onUpdate,
}: {
  state: DictionaryState;
  progress: DictionaryProgress | null;
  onUpdate: () => Promise<void>;
}) {
  const updating = state.status === "updating" || progress !== null;
  const title = state.status === "failed" ? "词典不可用" : state.status === "updating" ? "正在更新词典" : "安装离线词典";
  const description = state.status === "failed"
    ? state.error ?? "当前词典完整性检查未通过，可以重新下载。"
    : "词典数据单独存储在应用数据目录中，首次使用需要下载约 207 MB 工件。";
  return (
    <div className="dictionary-install-card">
      <div>
        <strong>{title}</strong>
        <p>{description}</p>
        {updating && (
          <div className="dictionary-progress" aria-live="polite">
            <div className="dictionary-progress-label"><span>{progressLabel(progress)}</span><span>{progress ? `${formatBytes(progress.current)} / ${formatBytes(progress.total)}` : ""}</span></div>
            <div className="dictionary-progress-track"><span style={{ width: `${progressPercent(progress)}%` }} /></div>
          </div>
        )}
      </div>
      <button className="secondary-button" type="button" onClick={() => void onUpdate()} disabled={updating}>
        {updating ? <LoaderCircle className="spin" size={15} /> : <Download size={15} />}
        {state.status === "failed" ? "重试下载" : "下载词典"}
      </button>
    </div>
  );
}

function DictionaryEntryView({
  entry,
  lookup,
  example,
  wordExample,
  onCancelExample,
  isFavorite,
  savingFavorite,
  onFavoriteToggle,
}: {
  entry: DictionaryEntry;
  lookup: DictionaryLookupResult | null;
  example: ParagraphExample | null;
  wordExample: WordExampleState;
  onCancelExample: () => void;
  isFavorite: boolean;
  savingFavorite: boolean;
  onFavoriteToggle: () => void;
}) {
  const pronunciations = collectPronunciations(entry).map(
    (pronunciation) => pronunciation.ipa ?? pronunciation.text,
  );
  return (
    <article className="dictionary-entry">
      <header className="dictionary-entry-header">
        <div>
          <div className="dictionary-entry-title-row">
            <h2>{entry.headword}</h2>
            <button
              className={`icon-button dictionary-favorite-button ${isFavorite ? "is-active" : ""}`}
              type="button"
              title={isFavorite ? "取消收藏" : "收藏词条"}
              aria-label={isFavorite ? "取消收藏" : "收藏词条"}
              onClick={onFavoriteToggle}
              disabled={savingFavorite}
            >
              <Bookmark size={18} fill={isFavorite ? "currentColor" : "none"} />
            </button>
          </div>
          {entry.headword_summary && <p>{entry.headword_summary}</p>}
          {lookup?.matchType === "form" && (
            <p className="dictionary-form-source">
              词形 {lookup.word} → 规范词头 {lookup.canonicalWord}
            </p>
          )}
          {example && (
            <div className="dictionary-source-example">
              <p>{example.sourceText}</p>
              {wordExample.exampleId === example.exampleId && (
                <div className="dictionary-ai-example">
                  {wordExample.translation && <p>{wordExample.translation}</p>}
                  {wordExample.partOfSpeech && <span>词性：{wordExample.partOfSpeech}</span>}
                  {wordExample.status === "streaming" && <span className="dictionary-ai-status">正在生成</span>}
                  {wordExample.status === "failed" && wordExample.error && <span className="error-message">{wordExample.error}</span>}
                  {wordExample.status === "streaming" && (
                    <button className="text-button" type="button" onClick={onCancelExample}>取消</button>
                  )}
                </div>
              )}
            </div>
          )}
        </div>
        {pronunciations.length > 0 && <div className="dictionary-pronunciations">{pronunciations.join(" · ")}</div>}
      </header>
      {entry.pos_groups.map((group, index) => <DictionaryPosGroupView key={`${group.pos}-${index}`} group={group} />)}
    </article>
  );
}

function DictionaryPosGroupView({ group }: { group: DictionaryPosGroup }) {
  const [showRare, setShowRare] = useState(false);
  const { visible, hidden } = splitMeaningsByPriority(group);
  const relations = groupRelationsByType(group);
  return (
    <section className="dictionary-pos-group">
      <div className="dictionary-pos-heading">
        <span className="dictionary-pos-en">{group.pos}</span>
        {posLabelZh(group.pos) && <span className="dictionary-pos-zh">{posLabelZh(group.pos)}</span>}
        {group.proper_name && <span className="dictionary-tag">专有名词</span>}
      </div>
      {group.summary && <p className="dictionary-pos-summary">{group.summary}</p>}
      <div className="dictionary-meanings">
        {visible.map((meaning, index) => <DictionaryMeaningView key={meaning.sense_id} meaning={meaning} index={index} />)}
        {showRare && hidden.map((meaning, index) => <DictionaryMeaningView key={meaning.sense_id} meaning={meaning} index={visible.length + index} />)}
      </div>
      {hidden.length > 0 && (
        <button className="text-button dictionary-rare-toggle" type="button" onClick={() => setShowRare((current) => !current)}>
          {showRare ? "隐藏罕见义项" : `显示 ${hidden.length} 条罕见义项`}
        </button>
      )}
      {[...relations.entries()].map(([type, words]) => (
        <div className="dictionary-relations" key={type}>
          <span>{relationLabel(type)}</span>
          <div>{words.map((word) => <span className="dictionary-relation-chip" key={word}>{word}</span>)}</div>
        </div>
      ))}
    </section>
  );
}

function DictionaryMeaningView({ meaning, index }: { meaning: DictionaryMeaning; index: number }) {
  return (
    <div className="dictionary-meaning">
      <div className="dictionary-meaning-title">
        <span className="dictionary-meaning-index">{index + 1}</span>
        {meaning.short_gloss && <strong>{meaning.short_gloss}</strong>}
        {meaning.priority === "core" && <span className="dictionary-core-dot" aria-label="核心义项" />}
        {meaning.priority === "rare" && <span className="dictionary-tag">罕见</span>}
        {meaning.labels.length > 0 && <span className="dictionary-labels">[{meaning.labels.join(", ")}]</span>}
      </div>
      <p>{meaning.learner_explanation}</p>
      {meaning.usage_note && <p className="dictionary-usage-note">用法：{meaning.usage_note}</p>}
      {meaning.examples.length > 0 && (
        <div className="dictionary-examples">
          {meaning.examples.map((example, exampleIndex) => (
            <div className="dictionary-example" key={`${meaning.sense_id}-${exampleIndex}`}>
              <p>{example.text}</p>
              <p>{example.translation}</p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function relationLabel(type: string): string {
  const labels: Record<string, string> = {
    synonym: "近义",
    antonym: "反义",
    related_term: "相关",
    derived_term: "派生",
  };
  return labels[type] ?? type;
}
