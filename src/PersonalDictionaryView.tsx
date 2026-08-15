import { useState } from "react";
import { Bookmark, Download, ExternalLink, Trash2 } from "lucide-react";
import type { PersonalDictionaryEntry } from "./types/contracts";

export default function PersonalDictionaryView({
  entries,
  onOpen,
  onRemove,
  onExport,
}: {
  entries: PersonalDictionaryEntry[];
  onOpen: (entry: PersonalDictionaryEntry) => void;
  onRemove: (entry: PersonalDictionaryEntry) => void;
  onExport: () => void;
}) {
  const [visibleCount, setVisibleCount] = useState(10);
  const visibleEntries = entries.slice(0, visibleCount);

  return (
    <section className="page-section narrow-page bounded-list-page personal-dictionary-page">
      <PageTitle onExport={onExport} />
      <div className="list-card bounded-list-card personal-dictionary-card">
        <div className="list-card-heading">
          <strong>已收藏词条</strong>
          <span>{entries.length} 条</span>
        </div>
        <div className="bounded-list-card-scroll">
          {entries.length === 0 ? (
            <div className="empty-list">在词典结果页收藏词条后，会显示在这里。</div>
          ) : (
            <>
              {visibleEntries.map((entry) => (
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
                    <button className="icon-button danger-icon-button" type="button" onClick={() => onRemove(entry)} title="移除收藏" aria-label="移除收藏"><Trash2 size={14} /></button>
                  </div>
                </div>
              ))}
              {visibleEntries.length < entries.length && <button className="list-load-more" type="button" onClick={() => setVisibleCount((current) => Math.min(current + 10, entries.length))}>更多</button>}
            </>
          )}
        </div>
      </div>
    </section>
  );
}

function PageTitle({ onExport }: { onExport: () => void }) {
  return (
    <div className="page-heading">
      <div>
        <p className="eyebrow">PERSONAL DICTIONARY</p>
        <h1>个人词典</h1>
      </div>
      <button className="secondary-button small-button" type="button" onClick={onExport}>
        <Download size={15} />
        导出词典
      </button>
    </div>
  );
}
