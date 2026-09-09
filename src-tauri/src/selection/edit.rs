//! Read-only support for the classic USER32 Edit control.
//!
//! Every message used here is below `WM_USER`, so USER32 can marshal it to a
//! remote process. RichEdit, Scintilla, and other controls with private
//! pointer-bearing messages are intentionally excluded.

use super::provider::{MAX_TEXT_UNITS, ReadOutcome, selected_utf16};
use super::{SelectionSourceContext, process_id_from_root, root_window_from_raw};
use windows::Win32::{
    Foundation::{HWND, LPARAM, POINT, WPARAM},
    UI::WindowsAndMessaging::{
        ES_PASSWORD, GUITHREADINFO, GWL_STYLE, GetClassNameW, GetGUIThreadInfo, GetWindowLongW,
        GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SMTO_BLOCK, SMTO_ERRORONEXIT,
        SendMessageTimeoutW, WM_GETTEXT, WM_GETTEXTLENGTH, WindowFromPoint,
    },
};

// EM_GETSEL is generated under Win32::UI::Controls in windows-rs, while this
// crate deliberately does not enable that broad feature set. It is a system
// message (< WM_USER), so retaining the numeric SDK value is safe here.
const EM_GETSEL: u32 = 0x00B0;
const MESSAGE_TIMEOUT_MS: u32 = 100;

fn same_control(window: HWND, context: &SelectionSourceContext) -> bool {
    root_window_from_raw(window.0 as isize) == Some(context.root_window)
        && process_id_from_root(window.0 as isize) == Some(context.process_id)
}

fn standard_edit(window: HWND) -> bool {
    let mut class = [0u16; 64];
    let length = unsafe { GetClassNameW(window, &mut class) };
    if length <= 0 {
        return false;
    }
    String::from_utf16(&class[..length as usize])
        .map(|name| name.eq_ignore_ascii_case("Edit"))
        .unwrap_or(false)
        && unsafe { GetWindowLongW(window, GWL_STYLE) } & ES_PASSWORD == 0
}

pub(super) fn is_standard_edit_window(raw_window: isize) -> bool {
    let window = HWND(raw_window as *mut core::ffi::c_void);
    !window.0.is_null() && standard_edit(window)
}

fn query(window: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> Result<usize, ReadOutcome> {
    let mut result = 0usize;
    let sent = unsafe {
        SendMessageTimeoutW(
            window,
            message,
            wparam,
            lparam,
            SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
            MESSAGE_TIMEOUT_MS,
            Some(&mut result),
        )
    };
    if sent.0 == 0 {
        Err(ReadOutcome::TimedOut)
    } else {
        Ok(result)
    }
}

fn selection(window: HWND) -> Result<(u32, u32), ReadOutcome> {
    let (mut start, mut end) = (0u32, 0u32);
    query(
        window,
        EM_GETSEL,
        WPARAM((&mut start as *mut u32) as usize),
        LPARAM((&mut end as *mut u32) as isize),
    )?;
    Ok((start, end))
}

fn selection_bounds(
    window: HWND,
    context: &SelectionSourceContext,
) -> Result<(u32, u32), ReadOutcome> {
    if !same_control(window, context) {
        return Err(ReadOutcome::Stale);
    }
    if !standard_edit(window) {
        return Err(ReadOutcome::Unsupported);
    }
    selection(window)
}

fn has_selection(window: HWND, context: &SelectionSourceContext) -> Result<bool, ReadOutcome> {
    let (start, end) = selection_bounds(window, context)?;
    if start == end {
        return Ok(false);
    }
    let length = query(window, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0))?;
    if length > MAX_TEXT_UNITS {
        return Err(ReadOutcome::Unsupported);
    }
    if start as usize > length || end as usize > length {
        return Err(ReadOutcome::Stale);
    }
    Ok(true)
}

fn read_control(window: HWND, context: &SelectionSourceContext) -> ReadOutcome {
    let read = || -> Result<ReadOutcome, ReadOutcome> {
        let (start, end) = selection_bounds(window, context)?;
        if start == end {
            return Ok(ReadOutcome::NoSelection);
        }

        let length = query(window, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0))?;
        if length > MAX_TEXT_UNITS {
            return Err(ReadOutcome::Unsupported);
        }
        if start as usize > length || end as usize > length {
            return Err(ReadOutcome::Stale);
        }

        let mut text = vec![0u16; length.saturating_add(1)];
        let copied = query(
            window,
            WM_GETTEXT,
            WPARAM(text.len()),
            LPARAM(text.as_mut_ptr() as isize),
        )?;
        if copied != length
            || selection_bounds(window, context)? != (start, end)
            || query(window, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0))? != length
        {
            return Err(ReadOutcome::Stale);
        }
        Ok(selected_utf16(&text[..copied], start, end))
    };
    read().unwrap_or_else(|outcome| outcome)
}

fn candidate_windows(
    context: &SelectionSourceContext,
    preferred_window: Option<isize>,
) -> Vec<HWND> {
    if let Some(raw_window) = preferred_window {
        let window = HWND(raw_window as *mut core::ffi::c_void);
        return (!window.0.is_null() && same_control(window, context) && standard_edit(window))
            .then_some(window)
            .into_iter()
            .collect();
    }

    let root = HWND(context.root_window as *mut core::ffi::c_void);
    let thread = unsafe { GetWindowThreadProcessId(root, None) };
    if thread == 0 {
        return Vec::new();
    }

    let focus = unsafe {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(thread, &mut info)
            .ok()
            .map(|_| info.hwndFocus)
    };
    let hit = context
        .release_point
        .map(|(x, y)| unsafe { WindowFromPoint(POINT { x, y }) });

    let mut candidates = Vec::with_capacity(3);
    if context.press_point.is_some() {
        // A drag belongs to the release window. A stale keyboard focus must
        // not make an unrelated Edit look like the selected source. If the
        // release window cannot be inspected, there is no safe Edit fallback.
        if let Some(hit) = hit {
            candidates.push(hit);
        }
    } else {
        // Shortcut reads have no drag geometry, so the keyboard focus is the
        // strongest source identity.
        candidates.extend(focus);
        candidates.extend(hit);
    }
    candidates.retain(|window| {
        !window.0.is_null() && same_control(*window, context) && standard_edit(*window)
    });
    candidates.dedup_by_key(|window| window.0 as isize);
    candidates
}

/// Find a standard Edit with a currently non-empty, internally consistent
/// selection. This probe reads only selection offsets and text length; it does
/// not read the control body until the user requests the result.
pub(super) fn find_selected_control(
    context: &SelectionSourceContext,
    preferred_window: Option<isize>,
) -> Option<isize> {
    candidate_windows(context, preferred_window)
        .into_iter()
        .find(|window| matches!(has_selection(*window, context), Ok(true)))
        .map(|window| window.0 as isize)
}

/// Read the selected UTF-16 slice from a standard Edit. A failed candidate is
/// not allowed to make the worker touch the clipboard or invoke another
/// control protocol; callers decide whether to continue with UIA.
pub(super) fn read_selection(
    context: &SelectionSourceContext,
    preferred_window: Option<isize>,
) -> ReadOutcome {
    let candidates = candidate_windows(context, preferred_window);
    if candidates.is_empty() {
        return ReadOutcome::Unsupported;
    }
    let mut outcome = ReadOutcome::Unsupported;
    for window in candidates {
        let current = read_control(window, context);
        match current {
            ReadOutcome::Selected(_) => return current,
            ReadOutcome::TimedOut => return current,
            ReadOutcome::Stale | ReadOutcome::Failed => outcome = current,
            ReadOutcome::NoSelection if matches!(outcome, ReadOutcome::Unsupported) => {
                outcome = current;
            }
            ReadOutcome::Unsupported | ReadOutcome::NoSelection => {}
        }
    }
    outcome
}
