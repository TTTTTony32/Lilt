import type { DictionaryInvalidInsight as DictionaryInvalidInsightData, DictionaryLookupCandidate } from "../types/dictionary";

interface DictionaryInvalidInsightProps {
  insight: DictionaryInvalidInsightData | null;
  onSuggestion: (candidate: DictionaryLookupCandidate) => void;
  compact?: boolean;
}

export default function DictionaryInvalidInsight({ insight, onSuggestion, compact = false }: DictionaryInvalidInsightProps) {
  if (!insight) return null;
  const hasSuggestions = insight.possibleSpellings.length > 0;
  const hasProperNoun = insight.properNoun !== null;
  const hasNote = insight.note.trim().length > 0;
  if (!hasSuggestions && !hasProperNoun && !hasNote) return null;

  return (
    <div className={`dictionary-invalid-insight${compact ? " is-compact" : ""}`}>
      <strong className="dictionary-invalid-insight-heading">AI 见解</strong>
      {hasNote && <p className="dictionary-invalid-insight-note">{insight.note}</p>}
      {hasSuggestions && (
        <div className="dictionary-invalid-insight-section">
          <span>可能的拼写</span>
          <div className="dictionary-invalid-insight-suggestions">
            {insight.possibleSpellings.map((candidate) => (
              <button
                className="dictionary-invalid-insight-button"
                key={candidate.normalizedCanonicalWord}
                type="button"
                onClick={() => onSuggestion(candidate)}
              >
                {candidate.canonicalWord}
              </button>
            ))}
          </div>
        </div>
      )}
      {hasProperNoun && insight.properNoun && (
        <div className="dictionary-invalid-insight-section">
          <span>可能的专有名词</span>
          <p><strong>{insight.properNoun.name}</strong>：{insight.properNoun.description}</p>
        </div>
      )}
    </div>
  );
}
