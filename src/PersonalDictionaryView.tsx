import { Bookmark, ExternalLink } from "lucide-react";
import type { PersonalDictionaryEntry } from "./types/contracts";

export default function PersonalDictionaryView({
  entries,
  onOpen,
  onRemove,
}: {
  entries: PersonalDictionaryEntry[];
  onOpen: (entry: PersonalDictionaryEntry) => void;
  onRemove: (entry: PersonalDictionaryEntry) => void;
}) {
  return (
    <section className="page-section narrow-page">
      <PageTitle />
      <div className="list-card">
        <div className="list-card-heading">
          <strong>已收藏词条</strong>
          <span>{entries.length} 条</span>
        </div>
        {entries.length === 0 ? (
          <div className="empty-list">在词典结果页收藏词条后，会显示在这里。</div>
        ) : (
          entries.map((entry) => (
            <div className="list-row personal-dictionary-row" key={entry.normalizedCanonicalWord}>
              <button className="personal-dictionary-open" type="button" onClick={() => onOpen(entry)}>
                <Bookmark size={15} fill="currentColor" />
                <span>
                  <strong>{entry.canonicalWord}</strong>
                  {entry.lookupWord.toLowerCase() !== entry.canonicalWord.toLowerCase() && <small>查询词形：{entry.lookupWord}</small>}
                </span>
              </button>
              <div className="button-group">
                <button className="text-button" type="button" onClick={() => onOpen(entry)} title="打开词典结果" aria-label="打开词典结果"><ExternalLink size={14} /></button>
                <button className="text-button danger-text" type="button" onClick={() => onRemove(entry)}>移除</button>
              </div>
            </div>
          ))
        )}
      </div>
    </section>
  );
}

function PageTitle() {
  return (
    <div className="page-heading">
      <div>
        <p className="eyebrow">PERSONAL DICTIONARY</p>
        <h1>个人词典</h1>
        <p className="page-description">保存常用词条，打开时读取当前离线词典内容。</p>
      </div>
    </div>
  );
}
