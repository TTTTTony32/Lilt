use crate::provider::{self, ProviderError};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranslationMode {
    Paragraph,
    PdfSegment,
    PdfPreflight,
    WordExample,
}

impl TranslationMode {
    pub fn from_wire_mode(mode: &str) -> Option<Self> {
        match mode {
            "paragraph" => Some(Self::Paragraph),
            "pdf_segment" => Some(Self::PdfSegment),
            "pdf_preflight" => Some(Self::PdfPreflight),
            "word_example" => Some(Self::WordExample),
            _ => None,
        }
    }

    pub const fn provider_operation(self) -> &'static str {
        match self {
            Self::Paragraph => "translate",
            Self::PdfSegment => "pdf_segment",
            Self::PdfPreflight => "pdf_preflight",
            Self::WordExample => "word_example",
        }
    }
}

pub struct StreamRequest<'a> {
    pub request_id: &'a str,
    pub base_url: &'a str,
    pub api_key: &'a str,
    pub model_id: &'a str,
    pub system_prompt: &'a str,
    pub user_text: &'a str,
    pub cancel: &'a CancellationToken,
    pub mode: TranslationMode,
    pub thinking_effort: &'a crate::contracts::ThinkingEffort,
}

pub struct TranslationCore;

impl TranslationCore {
    pub async fn stream<F>(request: StreamRequest<'_>, on_delta: F) -> Result<String, ProviderError>
    where
        F: FnMut(String) -> Result<(), ProviderError>,
    {
        Ok(Self::stream_with_usage(request, on_delta).await?.content)
    }

    pub async fn stream_with_usage<F>(
        request: StreamRequest<'_>,
        on_delta: F,
    ) -> Result<provider::ProviderStreamResult, ProviderError>
    where
        F: FnMut(String) -> Result<(), ProviderError>,
    {
        provider::stream_chat_completion_with_usage(
            provider::ChatStreamRequest {
                request_id: request.request_id,
                base_url: request.base_url,
                api_key: request.api_key,
                model_id: request.model_id,
                system_prompt: request.system_prompt,
                user_text: request.user_text,
                cancel: request.cancel,
                operation: request.mode.provider_operation(),
                thinking_effort: request.thinking_effort,
            },
            on_delta,
        )
        .await
    }

    pub async fn stream_with_usage_and_activity<F, A>(
        request: StreamRequest<'_>,
        on_delta: F,
        on_activity: A,
    ) -> Result<provider::ProviderStreamResult, ProviderError>
    where
        F: FnMut(String) -> Result<(), ProviderError>,
        A: FnMut(provider::ProviderStreamActivity),
    {
        provider::stream_chat_completion_with_usage_and_activity(
            provider::ChatStreamRequest {
                request_id: request.request_id,
                base_url: request.base_url,
                api_key: request.api_key,
                model_id: request.model_id,
                system_prompt: request.system_prompt,
                user_text: request.user_text,
                cancel: request.cancel,
                operation: request.mode.provider_operation(),
                thinking_effort: request.thinking_effort,
            },
            on_delta,
            on_activity,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::TranslationMode;

    #[test]
    fn modes_keep_a_single_provider_operation_mapping() {
        assert_eq!(
            TranslationMode::from_wire_mode("paragraph"),
            Some(TranslationMode::Paragraph)
        );
        assert_eq!(
            TranslationMode::from_wire_mode("pdf_segment"),
            Some(TranslationMode::PdfSegment)
        );
        assert_eq!(
            TranslationMode::from_wire_mode("pdf_preflight"),
            Some(TranslationMode::PdfPreflight)
        );
        assert_eq!(TranslationMode::from_wire_mode("unknown"), None);
        assert_eq!(TranslationMode::Paragraph.provider_operation(), "translate");
        assert_eq!(
            TranslationMode::PdfSegment.provider_operation(),
            "pdf_segment"
        );
        assert_eq!(
            TranslationMode::PdfPreflight.provider_operation(),
            "pdf_preflight"
        );
        assert_eq!(
            TranslationMode::WordExample.provider_operation(),
            "word_example"
        );
    }
}
