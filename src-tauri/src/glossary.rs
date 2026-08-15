use crate::contracts::GlossaryImportSkippedRow;
use csv::{ReaderBuilder, StringRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlossaryImportTerm {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedGlossaryImport {
    pub terms: Vec<GlossaryImportTerm>,
    pub skipped_rows: Vec<GlossaryImportSkippedRow>,
}

pub fn parse_csv(content: &str) -> ParsedGlossaryImport {
    let content = content.strip_prefix('\u{feff}').unwrap_or(content);
    let mut reader = ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(content.as_bytes());
    let mut terms = Vec::new();
    let mut skipped_rows = Vec::new();

    for (index, result) in reader.records().enumerate() {
        let line = index + 1;
        let record = match result {
            Ok(record) => record,
            Err(error) => {
                skipped_rows.push(GlossaryImportSkippedRow {
                    line,
                    reason: format!("CSV 结构无法解析：{error}"),
                });
                continue;
            }
        };

        if line == 1 && is_header(&record) {
            continue;
        }
        if record.len() != 2 {
            skipped_rows.push(GlossaryImportSkippedRow {
                line,
                reason: "需要原文和译文两个字段".to_string(),
            });
            continue;
        }

        let source = record.get(0).unwrap_or_default().trim().to_string();
        let target = record.get(1).unwrap_or_default().trim().to_string();
        if source.is_empty() {
            skipped_rows.push(GlossaryImportSkippedRow {
                line,
                reason: "原文不能为空".to_string(),
            });
            continue;
        }
        if target.is_empty() {
            skipped_rows.push(GlossaryImportSkippedRow {
                line,
                reason: "译文不能为空".to_string(),
            });
            continue;
        }

        if let Some(existing) = terms
            .iter_mut()
            .find(|term: &&mut GlossaryImportTerm| term.source == source)
        {
            existing.target = target;
        } else {
            terms.push(GlossaryImportTerm { source, target });
        }
    }

    ParsedGlossaryImport {
        terms,
        skipped_rows,
    }
}

fn is_header(record: &StringRecord) -> bool {
    if record.len() != 2 {
        return false;
    }
    let source = record.get(0).unwrap_or_default().trim();
    let target = record.get(1).unwrap_or_default().trim();
    (source == "原文" && target == "译文")
        || (source.eq_ignore_ascii_case("source") && target.eq_ignore_ascii_case("target"))
}

#[cfg(test)]
mod tests {
    use super::parse_csv;

    #[test]
    fn parses_bom_header_escaped_fields_and_keeps_last_duplicate_value() {
        let parsed = parse_csv(
            "\u{feff}原文,译文\n\"AI, model\",\"模型\"\nterm,first\nterm,\"last\nvalue\"\n",
        );

        assert!(parsed.skipped_rows.is_empty());
        assert_eq!(
            parsed.terms,
            vec![
                super::GlossaryImportTerm {
                    source: "AI, model".to_string(),
                    target: "模型".to_string(),
                },
                super::GlossaryImportTerm {
                    source: "term".to_string(),
                    target: "last\nvalue".to_string(),
                },
            ]
        );
    }

    #[test]
    fn accepts_data_without_a_header_and_reports_invalid_rows() {
        let parsed = parse_csv("actual,target\n,empty source\nvalid,\nvalid,target,extra\n");

        assert_eq!(parsed.terms.len(), 1);
        assert_eq!(parsed.terms[0].source, "actual");
        assert_eq!(parsed.skipped_rows.len(), 3);
        assert_eq!(parsed.skipped_rows[0].line, 2);
        assert_eq!(parsed.skipped_rows[1].line, 3);
        assert_eq!(parsed.skipped_rows[2].line, 4);
    }
}
