//! Selection retrieval adapted from selection-hook, Copyright (c) 2025 0xfullex.
//! Licensed under MIT; see licenses/selection-hook-MIT.txt.
use super::SelectionSourceContext;
use std::time::{Duration, Instant};
use uiautomation::{UIAutomation, UIElement};
use windows::{
    Win32::{
        Foundation::HWND,
        System::{
            Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx},
            Variant::{VARIANT, VT_ARRAY, VT_BSTR, VT_DISPATCH, VT_I4, VT_UNKNOWN},
        },
        UI::Accessibility::{
            AccessibleObjectFromWindow, CUIAutomation8, IAccessible, IUIAutomation, IUIAutomation2,
            IUIAutomationLegacyIAccessiblePattern, IUIAutomationTextPattern,
            UIA_E_TIMEOUT, UIA_LegacyIAccessiblePatternId, UIA_TextPatternId,
        },
    },
    core::Interface,
};

const PROVIDER_TIMEOUT_MS: u32 = 200;
const NATIVE_READ_BUDGET: Duration = Duration::from_millis(350);
const MAX_ANCESTORS: usize = 4;
const MAX_SELECTION_RANGES: i32 = 4;

fn call_failure(error: windows::core::Error, deadline: Instant) -> Outcome {
    if error.code().0 as u32 == UIA_E_TIMEOUT || Instant::now() >= deadline {
        Outcome::TimedOut
    } else {
        Outcome::Empty
    }
}

pub(super) fn create_automation() -> windows::core::Result<UIAutomation> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        // The older CUIAutomation class used by UIAutomation::new does not expose
        // provider timeouts. Keep slow providers from stalling the capture worker.
        let native: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        let settings = native.cast::<IUIAutomation2>()?;
        settings.SetConnectionTimeout(PROVIDER_TIMEOUT_MS)?;
        settings.SetTransactionTimeout(PROVIDER_TIMEOUT_MS)?;
        Ok(native.into())
    }
}

pub(super) enum Outcome {
    Text(String),
    Object,
    Empty,
    TimedOut,
}

fn text(value: String) -> Outcome {
    let value = clean_text(&value);
    if value.trim().is_empty() || value.encode_utf16().count() > 1_000_000 {
        Outcome::Empty
    } else {
        Outcome::Text(value)
    }
}

pub(super) fn clean_text(value: &str) -> String {
    let mut result = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{fffc}' {
            result.push(c);
            continue;
        }
        if result.ends_with('\n') {
            result.pop();
            if result.ends_with('\r') {
                result.pop();
            }
        } else if result.ends_with('\r') {
            result.pop();
        }
        if chars.peek() == Some(&'\r') {
            chars.next();
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
        } else if chars.peek() == Some(&'\n') {
            chars.next();
        }
    }
    result
}

unsafe fn accessible(accessible: &IAccessible, deadline: Instant) -> Outcome {
    if Instant::now() >= deadline {
        return Outcome::TimedOut;
    }
    let Ok(selection) = (unsafe { accessible.accSelection() }) else {
        return Outcome::Empty;
    };
    let raw = unsafe { &selection.Anonymous.Anonymous };
    if raw.vt == VT_BSTR {
        return text(unsafe { raw.Anonymous.bstrVal.to_string() });
    }
    if raw.vt == VT_UNKNOWN || raw.vt.0 & VT_ARRAY.0 != 0 {
        return Outcome::Object;
    }
    if Instant::now() >= deadline {
        return Outcome::TimedOut;
    }
    let role = if raw.vt == VT_I4 {
        unsafe { accessible.get_accRole(&selection) }.ok()
    } else if raw.vt == VT_DISPATCH {
        unsafe { &raw.Anonymous.pdispVal }
            .as_ref()
            .and_then(|d| d.cast::<IAccessible>().ok())
            .and_then(|a| unsafe { a.get_accRole(&VARIANT::from(0i32)) }.ok())
    } else {
        None
    };
    if let Some(role) = role {
        let raw = unsafe { &role.Anonymous.Anonymous };
        if raw.vt == VT_I4 {
            let role = unsafe { raw.Anonymous.lVal };
            if (1..=64).contains(&role) && role != 42 {
                return Outcome::Object;
            }
        }
    }
    if Instant::now() >= deadline {
        Outcome::TimedOut
    } else {
        Outcome::Empty
    }
}

fn pattern(element: &UIElement, deadline: Instant) -> Outcome {
    if Instant::now() >= deadline {
        return Outcome::TimedOut;
    }
    let native: &windows::Win32::UI::Accessibility::IUIAutomationElement = element.as_ref();
    let pattern = match unsafe {
        native.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
    } {
        Ok(pattern) => pattern,
        Err(error) => return call_failure(error, deadline),
    };
    if Instant::now() >= deadline {
        return Outcome::TimedOut;
    }
    let ranges = match unsafe { pattern.GetSelection() } {
        Ok(ranges) => ranges,
        Err(error) => return call_failure(error, deadline),
    };
    if Instant::now() >= deadline {
        return Outcome::TimedOut;
    }
    let count = match unsafe { ranges.Length() } {
        Ok(count) => count.min(MAX_SELECTION_RANGES),
        Err(error) => return call_failure(error, deadline),
    };
    for index in 0..count {
        if Instant::now() >= deadline {
            return Outcome::TimedOut;
        }
        let range = match unsafe { ranges.GetElement(index) } {
            Ok(range) => range,
            Err(error) => return call_failure(error, deadline),
        };
        if Instant::now() >= deadline {
            return Outcome::TimedOut;
        }
        match unsafe { range.GetText(1_000_001) } {
            Ok(value) => {
                let result = text(value.to_string());
                if matches!(result, Outcome::Text(_)) {
                    return result;
                }
            }
            Err(error) => return call_failure(error, deadline),
        }
    }
    // DocumentRange is the entire document, including on providers reporting an
    // active selection. Never serialize a browser page just to probe its selection.
    if Instant::now() >= deadline {
        Outcome::TimedOut
    } else {
        Outcome::Empty
    }
}

pub(super) fn read(automation: Option<&UIAutomation>, context: &SelectionSourceContext) -> Outcome {
    // Initialization failure must not switch to an unconfigured accessibility
    // client or start synchronous window MSAA calls. Let the caller copy instead.
    if automation.is_none() {
        return Outcome::Empty;
    }
    let deadline = Instant::now() + NATIVE_READ_BUDGET;
    let (result, msaa_read) = read_uia(automation, context, deadline);
    pipeline(
        || result,
        || {
            if Instant::now() >= deadline {
                Outcome::TimedOut
            } else if msaa_read {
                // The focused legacy bridge already queried this provider. A
                // second window-level MSAA round trip only repeats that work.
                Outcome::Empty
            } else {
                read_accessible(context, deadline)
            }
        },
    )
}

fn pipeline(uia: impl FnOnce() -> Outcome, msaa: impl FnOnce() -> Outcome) -> Outcome {
    let first = uia();
    if matches!(first, Outcome::Text(_) | Outcome::TimedOut) {
        return first;
    }
    let second = msaa();
    if matches!(second, Outcome::Text(_) | Outcome::TimedOut) {
        return second;
    }
    if matches!(first, Outcome::Object) || matches!(second, Outcome::Object) {
        Outcome::Object
    } else {
        Outcome::Empty
    }
}

fn read_uia(
    automation: Option<&UIAutomation>,
    context: &SelectionSourceContext,
    deadline: Instant,
) -> (Outcome, bool) {
    let mut object = false;
    let mut msaa_read = false;
    if let Some(automation) = automation {
        let focused = match automation.get_focused_element() {
            Ok(focused) => focused,
            Err(error) => return (call_failure(error.into(), deadline), false),
        };
        {
            if Instant::now() >= deadline {
                return (Outcome::TimedOut, false);
            }
            if focused.get_process_id().ok() == Some(context.process_id) {
                let result = pattern(&focused, deadline);
                if matches!(result, Outcome::Text(_) | Outcome::TimedOut) {
                    return (result, false);
                }
                // A focused web container often has no TextPattern; its Document
                // ancestor owns the selection. Try it before the legacy bridge.
                if Instant::now() >= deadline {
                    return (Outcome::TimedOut, false);
                }
                let native_automation: &IUIAutomation = automation.as_ref();
                if let Ok(walker) = unsafe { native_automation.ControlViewWalker() } {
                    let mut current = focused.clone();
                    for _ in 0..MAX_ANCESTORS {
                        if Instant::now() >= deadline {
                            return (Outcome::TimedOut, false);
                        }
                        let parent = match unsafe { walker.GetParentElement(current.as_ref()) } {
                            Ok(parent) => parent,
                            Err(error) => {
                                if matches!(call_failure(error, deadline), Outcome::TimedOut) {
                                    return (Outcome::TimedOut, false);
                                }
                                break;
                            }
                        };
                        let parent: UIElement = parent.into();
                        if Instant::now() >= deadline {
                            return (Outcome::TimedOut, false);
                        }
                        match parent.get_process_id() {
                            Ok(process_id) if process_id == context.process_id => {}
                            Ok(_) => break,
                            Err(error) => return (call_failure(error.into(), deadline), false),
                        }
                        let result = pattern(&parent, deadline);
                        if matches!(result, Outcome::Text(_) | Outcome::TimedOut) {
                            return (result, false);
                        }
                        current = parent;
                    }
                }
                if Instant::now() >= deadline {
                    return (Outcome::TimedOut, false);
                }
                let native: &windows::Win32::UI::Accessibility::IUIAutomationElement =
                    focused.as_ref();
                let legacy = match unsafe {
                    native.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                        UIA_LegacyIAccessiblePatternId,
                    )
                } {
                    Ok(legacy) => Some(legacy),
                    Err(error) => {
                        if matches!(call_failure(error, deadline), Outcome::TimedOut) {
                            return (Outcome::TimedOut, false);
                        }
                        None
                    }
                };
                if let Some(legacy) = legacy {
                    if Instant::now() >= deadline {
                        return (Outcome::TimedOut, false);
                    }
                    let legacy_accessible = match unsafe { legacy.GetIAccessible() } {
                        Ok(accessible) => Some(accessible),
                        Err(error) => {
                            if matches!(call_failure(error, deadline), Outcome::TimedOut) {
                                return (Outcome::TimedOut, false);
                            }
                            None
                        }
                    };
                    if let Some(a) = legacy_accessible {
                        msaa_read = true;
                        match unsafe { accessible(&a, deadline) } {
                            Outcome::Text(t) => return (Outcome::Text(t), true),
                            Outcome::Object => object = true,
                            Outcome::TimedOut => return (Outcome::TimedOut, true),
                            _ => {}
                        }
                    }
                }
            }
        }
    }
    if Instant::now() >= deadline {
        (Outcome::TimedOut, msaa_read)
    } else if object {
        (Outcome::Object, msaa_read)
    } else {
        (Outcome::Empty, msaa_read)
    }
}

fn read_accessible(context: &SelectionSourceContext, deadline: Instant) -> Outcome {
    let mut object = false;
    let mut raw = std::ptr::null_mut();
    if unsafe {
        AccessibleObjectFromWindow(
            HWND(context.root_window as *mut _),
            0xfffffffcu32,
            &IAccessible::IID,
            &mut raw,
        )
    }
    .is_ok()
        && !raw.is_null()
    {
        let a = unsafe { IAccessible::from_raw(raw) };
        match unsafe { accessible(&a, deadline) } {
            Outcome::Text(t) => return Outcome::Text(t),
            Outcome::Object => object = true,
            Outcome::TimedOut => return Outcome::TimedOut,
            _ => {}
        }
    }
    if Instant::now() >= deadline {
        Outcome::TimedOut
    } else if object {
        Outcome::Object
    } else {
        Outcome::Empty
    }
}

#[cfg(test)]
mod tests {
    use super::{Outcome, clean_text, pipeline};
    #[test]
    fn pipeline_short_circuits_text_and_retains_object_selections() {
        assert!(
            matches!(pipeline(||Outcome::Text("uia".into()),||panic!("MSAA must not run after UIA text")),Outcome::Text(t) if t=="uia")
        );
        assert!(
            matches!(pipeline(||Outcome::Object,||Outcome::Text("msaa".into())),Outcome::Text(t) if t=="msaa")
        );
        assert!(matches!(
            pipeline(|| Outcome::Object, || Outcome::Empty),
            Outcome::Object
        ));
        assert!(matches!(
            pipeline(|| Outcome::Empty, || Outcome::Object),
            Outcome::Object
        ));
    }
    #[test]
    fn removes_embedded_objects_and_only_adjacent_line_breaks() {
        assert_eq!(clean_text("a\r\n\u{fffc}\r\nb"), "ab");
        assert_eq!(clean_text("a\n\n\u{fffc}\n\nb"), "a\n\nb");
        assert_eq!(clean_text("a\u{fffc}b"), "ab");
    }
}
