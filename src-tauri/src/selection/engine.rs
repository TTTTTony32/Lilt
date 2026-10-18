//! Selection retrieval adapted from selection-hook, Copyright (c) 2025 0xfullex.
//! Licensed under MIT; see licenses/selection-hook-MIT.txt.
use super::SelectionSourceContext;
use uiautomation::{UIAutomation, UIElement, patterns::UITextPattern};
use windows::{
    Win32::{
        Foundation::HWND,
        System::Variant::{VARIANT, VT_ARRAY, VT_BSTR, VT_DISPATCH, VT_I4, VT_UNKNOWN},
        UI::Accessibility::{
            AccessibleObjectFromWindow, IAccessible, IUIAutomationLegacyIAccessiblePattern,
            UIA_LegacyIAccessiblePatternId,
        },
    },
    core::Interface,
};

pub(super) enum Outcome {
    Text(String),
    Object,
    Empty,
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

unsafe fn accessible(accessible: &IAccessible) -> Outcome {
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
    Outcome::Empty
}

fn pattern(element: &UIElement, allow_document: bool) -> Outcome {
    if let Ok(pattern) = element.get_pattern::<UITextPattern>() {
        if let Ok(ranges) = pattern.get_selection() {
            for range in ranges {
                if let Ok(value) = range.get_text(1_000_001) {
                    let result = text(value);
                    if matches!(result, Outcome::Text(_)) {
                        return result;
                    }
                }
            }
        }
        // Only the focus owner may use DocumentRange; ancestors expose explicit
        // selection ranges only, otherwise a whole document could be returned.
        if allow_document {
            if let Ok(range) = pattern.get_document_range() {
                let native: &windows::Win32::UI::Accessibility::IUIAutomationTextRange =
                    range.as_ref();
                if let Ok(active) = unsafe {
                    native.GetAttributeValue(
                        windows::Win32::UI::Accessibility::UIA_TEXTATTRIBUTE_ID(30034),
                    )
                } {
                    let raw = unsafe { &active.Anonymous.Anonymous };
                    if raw.vt == windows::Win32::System::Variant::VT_BOOL
                        && unsafe { raw.Anonymous.boolVal.as_bool() }
                    {
                        if let Ok(value) = range.get_text(1_000_001) {
                            return text(value);
                        }
                    }
                }
            }
        }
    }
    Outcome::Empty
}

pub(super) fn read(automation: Option<&UIAutomation>, context: &SelectionSourceContext) -> Outcome {
    pipeline(
        || read_uia(automation, context),
        || read_accessible(context),
    )
}

fn pipeline(uia: impl FnOnce() -> Outcome, msaa: impl FnOnce() -> Outcome) -> Outcome {
    let first = uia();
    if matches!(first, Outcome::Text(_)) {
        return first;
    }
    let second = msaa();
    if matches!(second, Outcome::Text(_)) {
        return second;
    }
    if matches!(first, Outcome::Object) || matches!(second, Outcome::Object) {
        Outcome::Object
    } else {
        Outcome::Empty
    }
}

fn read_uia(automation: Option<&UIAutomation>, context: &SelectionSourceContext) -> Outcome {
    let mut object = false;
    if let Some(automation) = automation {
        if let Ok(focused) = automation.get_focused_element() {
            if focused.get_process_id().ok() == Some(context.process_id) {
                let result = pattern(&focused, true);
                if matches!(result, Outcome::Text(_)) {
                    return result;
                }
                let native: &windows::Win32::UI::Accessibility::IUIAutomationElement =
                    focused.as_ref();
                if let Ok(legacy) = unsafe {
                    native.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
                        UIA_LegacyIAccessiblePatternId,
                    )
                } {
                    if let Ok(a) = unsafe { legacy.GetIAccessible() } {
                        match unsafe { accessible(&a) } {
                            Outcome::Text(t) => return Outcome::Text(t),
                            Outcome::Object => object = true,
                            _ => {}
                        }
                    }
                }
                if let Ok(walker) = automation.create_tree_walker() {
                    let mut current = focused;
                    for _ in 0..10 {
                        let Ok(parent) = walker.get_parent(&current) else {
                            break;
                        };
                        if parent.get_process_id().ok() != Some(context.process_id) {
                            break;
                        }
                        let result = pattern(&parent, false);
                        if matches!(result, Outcome::Text(_)) {
                            return result;
                        }
                        current = parent;
                    }
                }
            }
        }
    }
    if object {
        Outcome::Object
    } else {
        Outcome::Empty
    }
}

fn read_accessible(context: &SelectionSourceContext) -> Outcome {
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
        match unsafe { accessible(&a) } {
            Outcome::Text(t) => return Outcome::Text(t),
            Outcome::Object => object = true,
            _ => {}
        }
    }
    if object {
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
