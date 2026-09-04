use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const PARAGRAPH_LEARNING_PROTOCOL_VERSION: &str = "paragraph-learning-v2";
pub const PARAGRAPH_LEARNING_CACHE_MODE: &str = "paragraph-learning";
pub const MAX_PARAGRAPH_LEARNING_SEGMENTS: usize = 256;
pub const MAX_PARAGRAPH_LEARNING_ID_CHARS: usize = 128;
pub const MAX_PARAGRAPH_LEARNING_SOURCE_CHARS: usize = 100_000;
pub const MAX_PARAGRAPH_LEARNING_TRANSLATION_CHARS: usize = 100_000;
pub const MAX_PARAGRAPH_LEARNING_EXPLANATION_CHARS: usize = 2_000;
pub const MAX_PARAGRAPH_LEARNING_TOTAL_TRANSLATION_CHARS: usize = 200_000;

const MAX_SOURCE_MAPPING_SEARCH_NODES: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParagraphLearningResult {
    pub protocol_version: String,
    pub segments: Vec<ParagraphLearningSegment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParagraphLearningSegment {
    pub id: String,
    pub source: String,
    pub translation: String,
    pub explanation: String,
    pub source_start: usize,
    pub source_end: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelParagraphLearningResult {
    protocol_version: String,
    segments: Vec<ModelParagraphLearningSegment>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModelParagraphLearningSegment {
    id: String,
    source: String,
    translation: String,
    explanation: String,
}

impl ParagraphLearningResult {
    pub fn translated_text(&self) -> String {
        let total_length = self
            .segments
            .iter()
            .map(|segment| segment.translation.len())
            .sum();
        let mut translated = String::with_capacity(total_length);
        for segment in &self.segments {
            translated.push_str(&segment.translation);
        }
        translated
    }

    pub fn to_cache_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| format!("序列化段落学习结果失败：{error}"))
    }
}

pub fn build_system_prompt(base_prompt: &str) -> String {
    format!(
        "{base_prompt}\n\n段落学习模式结构化输出协议（版本 {PARAGRAPH_LEARNING_PROTOCOL_VERSION}）：只返回一个 JSON 对象，不要返回 Markdown 代码围栏、说明文字或其他内容。分段应按有实际学习价值的语言单位切分，优先使用单词（words）、词组或搭配（phrases or collocations），包括短语动词（phrasal verbs）、固定搭配（collocations）、习语（idioms）、名词短语（noun phrases）、动词短语（verb phrases）和简短语义片段或意群（short semantic chunks）等粒度，不要默认以完整句子作为一个分段。当一个句子包含多个可学习单位时，必须拆分为多个分段，禁止把完整句子单独作为一个分段。对象必须包含 protocolVersion（固定为 {PARAGRAPH_LEARNING_PROTOCOL_VERSION}）和 segments 数组。每个分段必须包含非空且唯一的 id、source、translation、explanation 字符串。source 必须是用户原文中的连续精确子串；所有 source 按原文连续覆盖全文，空格和标点必须原样保留，不得遗漏、重叠或改写。translation 按原文顺序拼接后构成完整译文，因此需要保留连接所需的空格和标点。explanation 使用简洁中文，只说明可验证的词义、语法、搭配、语气或文化背景，不输出完整推理过程。",
    )
}

pub fn parse_model_output(
    source_text: &str,
    model_output: &str,
) -> Result<ParagraphLearningResult, String> {
    let trimmed = model_output.trim();
    if trimmed.is_empty() {
        return Err(protocol_error("模型没有返回结构化内容"));
    }
    if trimmed.starts_with("```") || trimmed.ends_with("```") {
        return Err(protocol_error("不得使用 Markdown 代码围栏"));
    }
    let parsed = serde_json::from_str::<ModelParagraphLearningResult>(trimmed)
        .map_err(|error| protocol_error(format!("JSON 解析失败：{error}")))?;
    normalize_model_result(source_text, parsed)
}

pub fn parse_cached_result(
    source_text: &str,
    cached_json: &str,
) -> Result<ParagraphLearningResult, String> {
    let parsed = serde_json::from_str::<ParagraphLearningResult>(cached_json)
        .map_err(|error| protocol_error(format!("学习缓存 JSON 解析失败：{error}")))?;
    validate_normalized_result(source_text, parsed)
}

fn normalize_model_result(
    source_text: &str,
    parsed: ModelParagraphLearningResult,
) -> Result<ParagraphLearningResult, String> {
    if parsed.protocol_version != PARAGRAPH_LEARNING_PROTOCOL_VERSION {
        return Err(protocol_error(format!(
            "协议版本不匹配：{}",
            parsed.protocol_version
        )));
    }
    if parsed.segments.is_empty() {
        return Err(protocol_error("segments 不能为空"));
    }
    if parsed.segments.len() > MAX_PARAGRAPH_LEARNING_SEGMENTS {
        return Err(protocol_error(format!(
            "分段数量超过上限 {}",
            MAX_PARAGRAPH_LEARNING_SEGMENTS
        )));
    }
    if source_text.is_empty() {
        return Err(protocol_error("原文不能为空"));
    }

    let mut ids = HashSet::with_capacity(parsed.segments.len());
    let mut total_translation_chars = 0usize;
    for segment in &parsed.segments {
        validate_model_segment(segment)?;
        if !ids.insert(segment.id.clone()) {
            return Err(protocol_error(format!("包含重复分段 ID：{}", segment.id)));
        }
        total_translation_chars = total_translation_chars
            .checked_add(segment.translation.chars().count())
            .ok_or_else(|| protocol_error("译文长度超出可处理范围"))?;
    }
    if total_translation_chars > MAX_PARAGRAPH_LEARNING_TOTAL_TRANSLATION_CHARS {
        return Err(protocol_error(format!(
            "译文总长度超过上限 {}",
            MAX_PARAGRAPH_LEARNING_TOTAL_TRANSLATION_CHARS
        )));
    }

    let ranges = map_source_segments(source_text, &parsed.segments)?;
    let utf16_offsets = Utf16Offsets::new(source_text);
    let mut segments = parsed
        .segments
        .into_iter()
        .zip(ranges)
        .map(|(segment, (start, end))| {
            Ok(ParagraphLearningSegment {
                id: segment.id,
                source: segment.source,
                translation: segment.translation,
                explanation: segment.explanation,
                source_start: utf16_offsets
                    .for_byte_offset(start)
                    .ok_or_else(|| protocol_error("分段起点不是有效的 UTF-16 边界"))?,
                source_end: utf16_offsets
                    .for_byte_offset(end)
                    .ok_or_else(|| protocol_error("分段终点不是有效的 UTF-16 边界"))?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    segments.sort_by_key(|segment| segment.source_start);

    Ok(ParagraphLearningResult {
        protocol_version: PARAGRAPH_LEARNING_PROTOCOL_VERSION.to_string(),
        segments,
    })
}

fn validate_model_segment(segment: &ModelParagraphLearningSegment) -> Result<(), String> {
    if segment.id.trim().is_empty() {
        return Err(protocol_error("分段 ID 不能为空"));
    }
    if segment.id.chars().count() > MAX_PARAGRAPH_LEARNING_ID_CHARS {
        return Err(protocol_error(format!(
            "分段 ID 超过上限 {}",
            MAX_PARAGRAPH_LEARNING_ID_CHARS
        )));
    }
    if segment.source.is_empty() {
        return Err(protocol_error("原文分段不能为空"));
    }
    if segment.source.chars().count() > MAX_PARAGRAPH_LEARNING_SOURCE_CHARS {
        return Err(protocol_error(format!(
            "原文分段超过上限 {}",
            MAX_PARAGRAPH_LEARNING_SOURCE_CHARS
        )));
    }
    if segment.translation.is_empty() {
        return Err(protocol_error("分段译文不能为空"));
    }
    if segment.translation.chars().count() > MAX_PARAGRAPH_LEARNING_TRANSLATION_CHARS {
        return Err(protocol_error(format!(
            "分段译文超过上限 {}",
            MAX_PARAGRAPH_LEARNING_TRANSLATION_CHARS
        )));
    }
    if segment.explanation.trim().is_empty() {
        return Err(protocol_error("分段解释不能为空"));
    }
    if segment.explanation.chars().count() > MAX_PARAGRAPH_LEARNING_EXPLANATION_CHARS {
        return Err(protocol_error(format!(
            "分段解释超过上限 {}",
            MAX_PARAGRAPH_LEARNING_EXPLANATION_CHARS
        )));
    }
    Ok(())
}

fn validate_normalized_result(
    source_text: &str,
    mut result: ParagraphLearningResult,
) -> Result<ParagraphLearningResult, String> {
    if result.protocol_version != PARAGRAPH_LEARNING_PROTOCOL_VERSION {
        return Err(protocol_error(format!(
            "协议版本不匹配：{}",
            result.protocol_version
        )));
    }
    if result.segments.is_empty() {
        return Err(protocol_error("segments 不能为空"));
    }
    if result.segments.len() > MAX_PARAGRAPH_LEARNING_SEGMENTS {
        return Err(protocol_error(format!(
            "分段数量超过上限 {}",
            MAX_PARAGRAPH_LEARNING_SEGMENTS
        )));
    }
    if source_text.is_empty() {
        return Err(protocol_error("原文不能为空"));
    }

    let utf16_offsets = Utf16Offsets::new(source_text);
    let mut ids = HashSet::with_capacity(result.segments.len());
    let mut total_translation_chars = 0usize;
    let mut ranges = Vec::with_capacity(result.segments.len());
    for (index, segment) in result.segments.iter().enumerate() {
        validate_normalized_segment(segment)?;
        if !ids.insert(segment.id.clone()) {
            return Err(protocol_error(format!("包含重复分段 ID：{}", segment.id)));
        }
        total_translation_chars = total_translation_chars
            .checked_add(segment.translation.chars().count())
            .ok_or_else(|| protocol_error("译文长度超出可处理范围"))?;
        let start = utf16_offsets
            .to_byte_offset(segment.source_start)
            .ok_or_else(|| protocol_error(format!("分段 {index} 的起点不是有效范围")))?;
        let end = utf16_offsets
            .to_byte_offset(segment.source_end)
            .ok_or_else(|| protocol_error(format!("分段 {index} 的终点不是有效范围")))?;
        if start >= end {
            return Err(protocol_error(format!("分段 {index} 的范围为空或倒置")));
        }
        if source_text
            .get(start..end)
            .is_none_or(|source| source != segment.source)
        {
            return Err(protocol_error(format!(
                "分段 {index} 的范围与 source 不一致"
            )));
        }
        ranges.push((start, end, index));
    }
    if total_translation_chars > MAX_PARAGRAPH_LEARNING_TOTAL_TRANSLATION_CHARS {
        return Err(protocol_error(format!(
            "译文总长度超过上限 {}",
            MAX_PARAGRAPH_LEARNING_TOTAL_TRANSLATION_CHARS
        )));
    }

    ranges.sort_by_key(|(start, _, _)| *start);
    let mut expected_start = 0usize;
    for (start, end, _) in &ranges {
        if *start != expected_start {
            return Err(protocol_error("标准化分段未连续覆盖原文"));
        }
        expected_start = *end;
    }
    if expected_start != source_text.len() {
        return Err(protocol_error("标准化分段未覆盖完整原文"));
    }

    let sorted_segments = ranges
        .into_iter()
        .map(|(_, _, index)| result.segments[index].clone())
        .collect();
    result.segments = sorted_segments;
    Ok(result)
}

fn validate_normalized_segment(segment: &ParagraphLearningSegment) -> Result<(), String> {
    if segment.id.trim().is_empty() {
        return Err(protocol_error("分段 ID 不能为空"));
    }
    if segment.id.chars().count() > MAX_PARAGRAPH_LEARNING_ID_CHARS {
        return Err(protocol_error(format!(
            "分段 ID 超过上限 {}",
            MAX_PARAGRAPH_LEARNING_ID_CHARS
        )));
    }
    if segment.source.is_empty() {
        return Err(protocol_error("原文分段不能为空"));
    }
    if segment.source.chars().count() > MAX_PARAGRAPH_LEARNING_SOURCE_CHARS {
        return Err(protocol_error(format!(
            "原文分段超过上限 {}",
            MAX_PARAGRAPH_LEARNING_SOURCE_CHARS
        )));
    }
    if segment.translation.is_empty() {
        return Err(protocol_error("分段译文不能为空"));
    }
    if segment.translation.chars().count() > MAX_PARAGRAPH_LEARNING_TRANSLATION_CHARS {
        return Err(protocol_error(format!(
            "分段译文超过上限 {}",
            MAX_PARAGRAPH_LEARNING_TRANSLATION_CHARS
        )));
    }
    if segment.explanation.trim().is_empty() {
        return Err(protocol_error("分段解释不能为空"));
    }
    if segment.explanation.chars().count() > MAX_PARAGRAPH_LEARNING_EXPLANATION_CHARS {
        return Err(protocol_error(format!(
            "分段解释超过上限 {}",
            MAX_PARAGRAPH_LEARNING_EXPLANATION_CHARS
        )));
    }
    Ok(())
}

fn map_source_segments(
    source_text: &str,
    segments: &[ModelParagraphLearningSegment],
) -> Result<Vec<(usize, usize)>, String> {
    let total_source_bytes = segments
        .iter()
        .map(|segment| segment.source.len())
        .try_fold(0usize, |total, length| total.checked_add(length))
        .ok_or_else(|| protocol_error("原文分段长度超出可处理范围"))?;
    if total_source_bytes != source_text.len() {
        return Err(protocol_error("原文分段未完整覆盖输入原文"));
    }

    let mut used = vec![false; segments.len()];
    let mut bitset = vec![0u64; segments.len().div_ceil(64)];
    let mut ranges = vec![None; segments.len()];
    let mut failed_states = HashSet::new();
    let mut search_nodes = 0usize;
    let mapped = map_source_segments_inner(
        source_text,
        segments,
        0,
        total_source_bytes,
        &mut used,
        &mut bitset,
        &mut ranges,
        &mut failed_states,
        &mut search_nodes,
    );
    if !mapped {
        return Err(protocol_error("原文分段无法连续映射到输入原文"));
    }
    Ok(ranges
        .into_iter()
        .map(|range| range.expect("successful source mapping must assign every segment"))
        .collect())
}

#[allow(clippy::too_many_arguments)]
fn map_source_segments_inner(
    source_text: &str,
    segments: &[ModelParagraphLearningSegment],
    offset: usize,
    remaining_source_bytes: usize,
    used: &mut [bool],
    bitset: &mut [u64],
    ranges: &mut [Option<(usize, usize)>],
    failed_states: &mut HashSet<(usize, Vec<u64>)>,
    search_nodes: &mut usize,
) -> bool {
    *search_nodes += 1;
    if *search_nodes > MAX_SOURCE_MAPPING_SEARCH_NODES {
        return false;
    }
    if offset == source_text.len() {
        return remaining_source_bytes == 0 && used.iter().all(|value| *value);
    }
    if remaining_source_bytes != source_text.len().saturating_sub(offset) {
        return false;
    }
    if !failed_states.insert((offset, bitset.to_vec())) {
        return false;
    }

    let mut candidates = segments
        .iter()
        .enumerate()
        .filter(|(index, segment)| {
            !used[*index]
                && source_text[offset..].starts_with(&segment.source)
                && !segment.source.is_empty()
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        segments[*right]
            .source
            .len()
            .cmp(&segments[*left].source.len())
            .then_with(|| segments[*left].source.cmp(&segments[*right].source))
            .then_with(|| segments[*left].id.cmp(&segments[*right].id))
    });

    let mut previous_source = None;
    for index in candidates {
        let source = &segments[index].source;
        if previous_source == Some(source.as_str()) {
            continue;
        }
        previous_source = Some(source.as_str());
        let end = offset + source.len();
        used[index] = true;
        bitset[index / 64] |= 1u64 << (index % 64);
        ranges[index] = Some((offset, end));
        if map_source_segments_inner(
            source_text,
            segments,
            end,
            remaining_source_bytes - source.len(),
            used,
            bitset,
            ranges,
            failed_states,
            search_nodes,
        ) {
            return true;
        }
        used[index] = false;
        bitset[index / 64] &= !(1u64 << (index % 64));
        ranges[index] = None;
    }
    false
}

fn protocol_error(message: impl Into<String>) -> String {
    format!("段落学习协议无效：{}", message.into())
}

struct Utf16Offsets {
    boundaries: Vec<(usize, usize)>,
}

impl Utf16Offsets {
    fn new(source_text: &str) -> Self {
        let mut boundaries = vec![(0, 0)];
        let mut utf16_offset = 0;
        for (byte_offset, character) in source_text.char_indices() {
            utf16_offset += character.len_utf16();
            boundaries.push((utf16_offset, byte_offset + character.len_utf8()));
        }
        Self { boundaries }
    }

    fn for_byte_offset(&self, byte_offset: usize) -> Option<usize> {
        self.boundaries
            .iter()
            .find_map(|(utf16_offset, candidate)| {
                (*candidate == byte_offset).then_some(*utf16_offset)
            })
    }

    fn to_byte_offset(&self, utf16_offset: usize) -> Option<usize> {
        self.boundaries
            .binary_search_by_key(&utf16_offset, |(offset, _)| *offset)
            .ok()
            .map(|index| self.boundaries[index].1)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_PARAGRAPH_LEARNING_EXPLANATION_CHARS, MAX_PARAGRAPH_LEARNING_SEGMENTS,
        PARAGRAPH_LEARNING_PROTOCOL_VERSION, ParagraphLearningResult, build_system_prompt,
        parse_cached_result, parse_model_output,
    };
    use serde_json::json;

    fn valid_json() -> String {
        serde_json::to_string(&json!({
            "protocolVersion": PARAGRAPH_LEARNING_PROTOCOL_VERSION,
            "segments": [
                {
                    "id": "segment-1",
                    "source": "A ",
                    "translation": "一 ",
                    "explanation": "冠词与空格"
                },
                {
                    "id": "segment-2",
                    "source": "😀 test!",
                    "translation": "测试！",
                    "explanation": "示例短语"
                }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn normalizes_source_ranges_as_utf16_and_concatenates_translation() {
        let result = parse_model_output("A 😀 test!", &valid_json()).unwrap();
        assert_eq!(result.translated_text(), "一 测试！");
        assert_eq!(result.segments[0].source_start, 0);
        assert_eq!(result.segments[0].source_end, 2);
        assert_eq!(result.segments[1].source_start, 2);
        assert_eq!(result.segments[1].source_end, 10);
    }

    #[test]
    fn maps_reordered_model_segments_by_source_ranges() {
        let output = serde_json::to_string(&json!({
            "protocolVersion": PARAGRAPH_LEARNING_PROTOCOL_VERSION,
            "segments": [
                {
                    "id": "second",
                    "source": "world",
                    "translation": "世界",
                    "explanation": "名词"
                },
                {
                    "id": "first",
                    "source": "Hello ",
                    "translation": "你好 ",
                    "explanation": "问候语"
                }
            ]
        }))
        .unwrap();
        let result = parse_model_output("Hello world", &output).unwrap();
        assert_eq!(
            result
                .segments
                .iter()
                .map(|segment| segment.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
    }

    #[test]
    fn rejects_wrong_version_duplicate_ids_missing_coverage_and_overlap() {
        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["protocolVersion"] = json!("paragraph-learning-v0");
        assert!(parse_model_output("A 😀 test!", &value.to_string()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["segments"][1]["id"] = json!("segment-1");
        assert!(parse_model_output("A 😀 test!", &value.to_string()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["segments"][1]["source"] = json!("test!");
        assert!(parse_model_output("A 😀 test!", &value.to_string()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["segments"][1]["source"] = json!("😀 test");
        assert!(parse_model_output("A 😀 test!", &value.to_string()).is_err());
    }

    #[test]
    fn rejects_code_fences_wrong_types_unknown_fields_and_empty_explanations() {
        assert!(
            parse_model_output("A 😀 test!", &format!("```json\n{}\n```", valid_json())).is_err()
        );

        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["segments"][0]["translation"] = json!(1);
        assert!(parse_model_output("A 😀 test!", &value.to_string()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["extra"] = json!(true);
        assert!(parse_model_output("A 😀 test!", &value.to_string()).is_err());

        let mut value: serde_json::Value = serde_json::from_str(&valid_json()).unwrap();
        value["segments"][0]["explanation"] = json!(" ");
        assert!(parse_model_output("A 😀 test!", &value.to_string()).is_err());
    }

    #[test]
    fn rejects_limits_and_invalid_cached_ranges() {
        let long_explanation = "x".repeat(MAX_PARAGRAPH_LEARNING_EXPLANATION_CHARS + 1);
        let value = json!({
            "protocolVersion": PARAGRAPH_LEARNING_PROTOCOL_VERSION,
            "segments": [{
                "id": "segment-1",
                "source": "A",
                "translation": "一",
                "explanation": long_explanation
            }]
        });
        assert!(parse_model_output("A", &value.to_string()).is_err());

        let too_many = (0..=MAX_PARAGRAPH_LEARNING_SEGMENTS)
            .map(|index| {
                json!({
                    "id": format!("segment-{index}"),
                    "source": "A",
                    "translation": "一",
                    "explanation": "说明"
                })
            })
            .collect::<Vec<_>>();
        assert!(
            parse_model_output(
                &"A".repeat(MAX_PARAGRAPH_LEARNING_SEGMENTS + 1),
                &json!({
                    "protocolVersion": PARAGRAPH_LEARNING_PROTOCOL_VERSION,
                    "segments": too_many
                })
                .to_string()
            )
            .is_err()
        );

        let valid = parse_model_output("A 😀 test!", &valid_json()).unwrap();
        let mut cached: serde_json::Value = serde_json::to_value(valid).unwrap();
        cached["segments"][1]["sourceEnd"] = json!(999);
        assert!(parse_cached_result("A 😀 test!", &cached.to_string()).is_err());
    }

    #[test]
    fn cached_result_round_trips_only_normalized_fields() {
        let result = parse_model_output("A 😀 test!", &valid_json()).unwrap();
        let encoded = result.to_cache_json().unwrap();
        let decoded: ParagraphLearningResult = parse_cached_result("A 😀 test!", &encoded).unwrap();
        assert_eq!(decoded, result);
    }

    #[test]
    fn prompt_includes_versioned_json_contract() {
        let prompt = build_system_prompt("base prompt");
        assert!(prompt.starts_with("base prompt"));
        assert!(prompt.contains(PARAGRAPH_LEARNING_PROTOCOL_VERSION));
        assert!(prompt.contains("protocolVersion"));
        assert!(prompt.contains("segments 数组"));
        assert!(prompt.contains("id、source、translation、explanation 字符串"));
        assert!(prompt.contains("source"));
        assert!(prompt.contains("translation"));
        assert!(prompt.contains("explanation"));
    }

    #[test]
    fn prompt_requires_meaningful_learning_units_and_preserves_text_boundaries() {
        let prompt = build_system_prompt("base prompt");

        for unit in [
            "单词（words）",
            "词组或搭配（phrases or collocations）",
            "短语动词（phrasal verbs）",
            "固定搭配（collocations）",
            "习语（idioms）",
            "名词短语（noun phrases）",
            "动词短语（verb phrases）",
            "简短语义片段或意群（short semantic chunks）",
        ] {
            assert!(
                prompt.contains(unit),
                "prompt missing learning unit: {unit}"
            );
        }
        assert!(prompt.contains("semantic chunks"));
        assert!(prompt.contains("意群"));
        assert!(prompt.contains("不要默认以完整句子作为一个分段"));
        assert!(prompt.contains("当一个句子包含多个可学习单位时，必须拆分为多个分段"));
        assert!(prompt.contains("禁止把完整句子单独作为一个分段"));
        assert!(prompt.contains("所有 source 按原文连续覆盖全文"));
        assert!(prompt.contains("空格和标点必须原样保留"));
        assert!(prompt.contains("translation 按原文顺序拼接后构成完整译文"));
        assert!(prompt.contains("保留连接所需的空格和标点"));
    }
}
