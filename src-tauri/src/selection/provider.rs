//! Outcomes shared by readonly providers. No outcome contains diagnostic text.
//!
//! Providers deliberately return a small, closed set of states. The selection
//! worker owns the user-facing error wording; a native provider must not leak
//! arbitrary COM or Win32 error text into the selection event protocol.
#[derive(Debug)]
pub(super) enum ReadOutcome {
    Selected(String),
    NoSelection,
    Unsupported,
    TimedOut,
    Stale,
    Failed,
}

pub(super) const MAX_TEXT_UNITS: usize = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadOutcomeKind {
    Selected,
    NoSelection,
    Unsupported,
    TimedOut,
    Stale,
    Failed,
}

impl ReadOutcome {
    pub(super) fn kind(&self) -> ReadOutcomeKind {
        match self {
            Self::Selected(_) => ReadOutcomeKind::Selected,
            Self::NoSelection => ReadOutcomeKind::NoSelection,
            Self::Unsupported => ReadOutcomeKind::Unsupported,
            Self::TimedOut => ReadOutcomeKind::TimedOut,
            Self::Stale => ReadOutcomeKind::Stale,
            Self::Failed => ReadOutcomeKind::Failed,
        }
    }
}

/// Return the stable protocol code for a provider outcome.
pub(super) fn outcome_code(outcome: &ReadOutcome) -> &'static str {
    match outcome.kind() {
        ReadOutcomeKind::Selected => "selection_source_selected",
        ReadOutcomeKind::NoSelection => "no_selection",
        ReadOutcomeKind::Unsupported => "selection_source_unsupported",
        ReadOutcomeKind::TimedOut => "selection_read_timeout",
        ReadOutcomeKind::Stale => "selection_stale",
        ReadOutcomeKind::Failed => "selection_read_failed",
    }
}

/// Return bounded, provider-independent wording for a provider outcome.
pub(super) fn outcome_message(outcome: &ReadOutcome) -> &'static str {
    match outcome.kind() {
        ReadOutcomeKind::Selected => "文本选区读取成功",
        ReadOutcomeKind::NoSelection => "当前没有可读取的文本选区",
        ReadOutcomeKind::Unsupported => "当前窗口不支持安全的文本选区读取",
        ReadOutcomeKind::TimedOut => "文本选区读取超时",
        ReadOutcomeKind::Stale => "文本选区在读取期间已经失效",
        ReadOutcomeKind::Failed => "文本选区读取失败",
    }
}

/// Pick the most actionable failure when several independent providers fail.
/// A timeout or invalidated range is more useful than a generic unsupported
/// result, while an all-provider empty selection remains `no_selection`.
pub(super) fn preferred_failure<'a>(
    outcomes: impl IntoIterator<Item = &'a ReadOutcome>,
) -> Option<&'a ReadOutcome> {
    outcomes
        .into_iter()
        .max_by_key(|outcome| match outcome.kind() {
            ReadOutcomeKind::Selected => 0,
            ReadOutcomeKind::NoSelection => 1,
            ReadOutcomeKind::Unsupported => 2,
            ReadOutcomeKind::Failed => 3,
            ReadOutcomeKind::Stale => 4,
            ReadOutcomeKind::TimedOut => 5,
        })
}

pub(super) fn selected_utf16(text: &[u16], start: u32, end: u32) -> ReadOutcome {
    let (start, end) = (start.min(end) as usize, start.max(end) as usize);
    if start == end {
        return ReadOutcome::NoSelection;
    }
    let Some(slice) = text.get(start..end) else {
        return ReadOutcome::Stale;
    };
    match String::from_utf16(slice) {
        Ok(value) if !value.trim().is_empty() => ReadOutcome::Selected(value),
        Ok(_) => ReadOutcome::NoSelection,
        Err(_) => ReadOutcome::Stale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_selection_preserves_direction_crlf_and_full_offsets() {
        let mut text = vec![b'x' as u16; 70_000];
        text.extend("A😀\r\n尾".encode_utf16());
        assert!(matches!(
            selected_utf16(&text, 70_007, 70_001),
            ReadOutcome::Stale
        ));
        assert!(
            matches!(selected_utf16(&text, 70_006, 70_001), ReadOutcome::Selected(s) if s == "😀\r\n尾")
        );
        assert!(matches!(
            selected_utf16(&text, 70_002, 70_003),
            ReadOutcome::Stale
        ));
        assert!(matches!(
            selected_utf16(&text, 2, 2),
            ReadOutcome::NoSelection
        ));
    }

    #[test]
    fn outcome_mapping_stays_within_the_readonly_protocol() {
        assert_eq!(
            outcome_code(&ReadOutcome::Selected(String::new())),
            "selection_source_selected"
        );
        assert_eq!(outcome_code(&ReadOutcome::NoSelection), "no_selection");
        assert_eq!(
            outcome_code(&ReadOutcome::Unsupported),
            "selection_source_unsupported"
        );
        assert_eq!(
            outcome_code(&ReadOutcome::TimedOut),
            "selection_read_timeout"
        );
        assert_eq!(outcome_code(&ReadOutcome::Stale), "selection_stale");
        assert_eq!(outcome_code(&ReadOutcome::Failed), "selection_read_failed");
        assert!(!outcome_message(&ReadOutcome::TimedOut).is_empty());
    }

    #[test]
    fn preferred_failure_prioritizes_timeout_over_empty_selection() {
        let outcomes = [
            ReadOutcome::NoSelection,
            ReadOutcome::Unsupported,
            ReadOutcome::TimedOut,
        ];
        assert!(matches!(
            preferred_failure(outcomes.iter()),
            Some(ReadOutcome::TimedOut)
        ));
    }
}
