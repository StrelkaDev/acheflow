//! AppKit field editors are attached above the active Blitz text box.
use crate::text_input::{Bridge, Field, Rect, Target};
use dioxus_native::{NodeHandle, prelude::*};
use objc2::{
    MainThreadMarker, MainThreadOnly, define_class, extern_methods,
    rc::Retained,
    runtime::{ProtocolObject, Sel},
    sel,
};
use objc2_app_kit::{
    NSApplication, NSColor, NSControlTextEditingDelegate, NSEvent, NSFocusRingType, NSFont, NSMenu,
    NSMenuItem, NSSecureTextField, NSText, NSTextDelegate, NSTextField, NSTextFieldDelegate,
    NSTextInputClient, NSTextView, NSTextViewDelegate, NSView, NSWindow,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRange, NSRect, NSSize, NSString,
};
use std::{cell::RefCell, rc::Rc};

thread_local! {
    static EDIT: RefCell<Option<Editing>> = const { RefCell::new(None) };
}

struct Editing {
    bridge: Rc<Bridge>,
    target: Target,
    last: String,
    limit: Option<usize>,
    action: Option<&'static str>,
}

fn current_text(view: &NSText) -> Option<String> {
    if let Some(editor) = view.downcast_ref::<NSTextView>()
        && editor.hasMarkedText()
    {
        return None;
    }
    Some(view.string().to_string())
}

fn publish(value: String) {
    let edit = EDIT.with_borrow_mut(|state| {
        let state = state.as_mut()?;
        let value = limit_utf16(value, state.limit);
        if state.last == value || state.action.is_some() {
            return None;
        }
        state.last.clone_from(&value);
        Some((state.bridge.clone(), state.target, value))
    });
    if let Some((bridge, target, value)) = edit {
        bridge.update(target, value);
    }
}

fn limit_utf16(mut value: String, limit: Option<usize>) -> String {
    if let Some(limit) = limit {
        let mut units = 0;
        if let Some((end, _)) = value.char_indices().find(|&(_, ch)| {
            units += ch.len_utf16();
            units > limit
        }) {
            value.truncate(end);
        }
    }
    value
}

fn field_changed(notification: &NSNotification) {
    if let Some(field) = notification
        .object()
        .and_then(|object| object.downcast::<NSTextField>().ok())
    {
        if let Some(editor) = field.currentEditor() {
            if let Some(value) = current_text(&editor) {
                publish(value);
            }
        } else {
            publish(field.stringValue().to_string());
        }
    }
}

fn text_changed(notification: &NSNotification) {
    if let Some(view) = notification
        .object()
        .and_then(|object| object.downcast::<NSTextView>().ok())
        && let Some(value) = current_text(&view)
    {
        publish(value);
    }
}

fn set_action(action: &'static str) {
    EDIT.with_borrow_mut(|state| {
        if let Some(state) = state {
            state.action.get_or_insert(action);
        }
    });
}

fn command(selector: Sel, multiline: bool) -> bool {
    let action = if selector == sel!(insertTab:) {
        "tab"
    } else if selector == sel!(insertBacktab:) {
        "backtab"
    } else if selector == sel!(cancelOperation:) {
        "escape"
    } else if selector == sel!(insertNewline:) && !multiline {
        "enter"
    } else {
        return false;
    };
    set_action(action);
    true
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AcheflowTextInputDelegate"]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSTextFieldDelegate for Delegate {}
    unsafe impl NSControlTextEditingDelegate for Delegate {
        #[unsafe(method(controlTextDidChange:))]
        fn control_changed(&self, event: &NSNotification) {
            field_changed(event);
        }

        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_ended(&self, event: &NSNotification) {
            field_changed(event);
            set_action("blur");
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_command(
            &self,
            _control: &objc2_app_kit::NSControl,
            text: &NSTextView,
            selector: Sel,
        ) -> bool {
            !text.hasMarkedText() && command(selector, false)
        }
    }

    unsafe impl NSTextDelegate for Delegate {
        #[unsafe(method(textDidChange:))]
        fn text_changed(&self, event: &NSNotification) {
            text_changed(event);
        }

        #[unsafe(method(textDidEndEditing:))]
        fn text_ended(&self, event: &NSNotification) {
            text_changed(event);
            set_action("blur");
        }
    }
    unsafe impl NSTextViewDelegate for Delegate {
        #[unsafe(method(textView:doCommandBySelector:))]
        fn text_command(&self, text: &NSTextView, selector: Sel) -> bool {
            !text.hasMarkedText() && command(selector, true)
        }
    }
);
impl Delegate {
    extern_methods!(
        #[unsafe(method(new))]
        fn new(mtm: MainThreadMarker) -> Retained<Self>;
    );
}

// The delegate is held by Editor and outlives the attached view. These
// subclasses expose AppKit's weak setter only to the editor construction path.
macro_rules! line_editor {
    ($name:ident, $parent:ty, $objc_name:literal, $secret:literal) => {
        define_class!(
            #[unsafe(super($parent))]
            #[thread_kind = MainThreadOnly]
            #[name = $objc_name]
            struct $name;
            impl $name {
                #[unsafe(method_id(menuForEvent:))]
                fn menu_for_event(&self, _event: &NSEvent) -> Option<Retained<NSMenu>> {
                    Some(edit_menu(self.mtm(), $secret))
                }
            }
        );
        impl $name {
            extern_methods!(
                #[unsafe(method(initWithFrame:))]
                fn init(this: objc2::rc::Allocated<Self>, frame: NSRect) -> Retained<Self>;

                #[unsafe(method(setDelegate:))]
                fn install_delegate(&self, delegate: &ProtocolObject<dyn NSTextFieldDelegate>);
            );
        }
    };
}
line_editor!(LineEditor, NSTextField, "AcheflowLineEditor", false);
line_editor!(
    PasswordEditor,
    NSSecureTextField,
    "AcheflowPasswordEditor",
    true
);

define_class!(
    #[unsafe(super(NSMenuItem))]
    #[thread_kind = MainThreadOnly]
    #[name = "AcheflowEditMenuItem"]
    struct EditMenuItem;
);

impl EditMenuItem {
    extern_methods!(
        // Callers use only the AppKit first-responder text actions below.
        #[unsafe(method(initWithTitle:action:keyEquivalent:))]
        fn init(
            this: objc2::rc::Allocated<Self>,
            title: &NSString,
            action: Option<Sel>,
            key: &NSString,
        ) -> Retained<Self>;
    );
}

fn edit_menu(mtm: MainThreadMarker, secret: bool) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(mtm.alloc(), &NSString::from_str("Edit"));
    for (title, selector, key) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        if secret && matches!(key, "x" | "c") {
            continue;
        }
        let item = EditMenuItem::init(
            mtm.alloc(),
            &NSString::from_str(title),
            Some(selector),
            &NSString::from_str(key),
        );
        menu.addItem(&item);
    }
    menu
}

fn install_edit_menu(app: &NSApplication, mtm: MainThreadMarker) {
    let title = NSString::from_str("Edit");
    if let Some(bar) = app.mainMenu()
        && bar.itemWithTitle(&title).is_none()
    {
        let item = NSMenuItem::new(mtm);
        item.setTitle(&title);
        item.setSubmenu(Some(&edit_menu(mtm, false)));
        bar.addItem(&item);
    }
}

struct Editor {
    field: Field,
    window: Retained<NSWindow>,
    parent: Retained<NSView>,
    view: Retained<NSView>,
    _delegate: Retained<Delegate>,
    rect: Rect,
}

impl Editor {
    fn open(
        mtm: MainThreadMarker,
        bridge: Rc<Bridge>,
        window: Retained<NSWindow>,
        field: Field,
    ) -> Option<Self> {
        let parent = window.contentView()?;
        let rect = field.rect;
        let frame = frame_in(&parent, editor_rect(&field, rect));
        let delegate = Delegate::new(mtm);
        let background = NSColor::colorWithSRGBRed_green_blue_alpha(0.059, 0.086, 0.125, 1.);
        let foreground = NSColor::colorWithSRGBRed_green_blue_alpha(0.894, 0.918, 0.957, 1.);
        let selection = NSRange::new(field.start, field.end.saturating_sub(field.start));
        let string = NSString::from_str(&field.value);
        EDIT.with_borrow_mut(|state| {
            *state = Some(Editing {
                bridge,
                target: field.target,
                last: field.value.clone(),
                limit: field.max_length,
                action: None,
            })
        });
        let view: Retained<NSView> = if field.multiline {
            let scroll = NSTextView::scrollableTextView(mtm);
            let Some(text) = scroll
                .documentView()
                .and_then(|view| view.downcast::<NSTextView>().ok())
            else {
                EDIT.with_borrow_mut(|state| *state = None);
                return None;
            };
            scroll.setFrame(frame);
            text.setRichText(false);
            text.setAllowsUndo(true);
            text.setString(&string);
            text.setBackgroundColor(&background);
            text.setTextColor(Some(&foreground));
            text.setInsertionPointColor(Some(&foreground));
            let font = if field.code {
                NSFont::userFixedPitchFontOfSize(12.)
                    .unwrap_or_else(|| NSFont::systemFontOfSize(12.))
            } else {
                NSFont::systemFontOfSize(13.)
            };
            text.setFont(Some(&font));
            text.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            parent.addSubview(&scroll);
            window.makeFirstResponder(Some(&text));
            text.setSelectedRange(selection);
            scroll.into_super()
        } else {
            let protocol = ProtocolObject::from_ref(&*delegate);
            let control = if field.secret {
                let control = PasswordEditor::init(mtm.alloc(), frame);
                control.install_delegate(protocol);
                control.into_super().into_super()
            } else {
                let control = LineEditor::init(mtm.alloc(), frame);
                control.install_delegate(protocol);
                control.into_super()
            };
            control.setEditable(true);
            control.setSelectable(true);
            control.setBezeled(false);
            control.setBordered(false);
            control.setFocusRingType(NSFocusRingType::None);
            control.setDrawsBackground(true);
            control.setBackgroundColor(Some(&background));
            control.setTextColor(Some(&foreground));
            control.setFont(Some(&NSFont::systemFontOfSize(13.)));
            control.setStringValue(&string);
            control.setPlaceholderString(Some(&NSString::from_str(&field.placeholder)));
            parent.addSubview(&control);
            window.makeFirstResponder(Some(&control));
            if let Some(text) = control.currentEditor() {
                text.setSelectedRange(selection);
            }
            control.into_super().into_super()
        };
        Some(Self {
            field,
            window,
            parent,
            view,
            _delegate: delegate,
            rect,
        })
    }

    fn update_position(&mut self, rect: Rect) {
        let frame = frame_in(&self.parent, editor_rect(&self.field, rect));
        if self.rect != rect || self.view.frame() != frame {
            self.view.setFrame(frame);
            self.rect = rect;
        }
    }

    fn finish(self, action: &str) {
        let Some(state) = EDIT.with_borrow_mut(Option::take) else {
            return;
        };
        if self.window.isKeyWindow() {
            self.window.makeFirstResponder(Some(&self.parent));
        }
        let mut value = state.last;
        if action == "escape" && value != self.field.value {
            value = self.field.value.clone();
            state.bridge.update(state.target, value.clone());
        }
        state.bridge.finish(state.target, value, action);
    }
}

impl Drop for Editor {
    fn drop(&mut self) {
        EDIT.with_borrow_mut(|state| *state = None);
        self.view.removeFromSuperview();
    }
}

// The canvas draws the border. The system control covers just the same text
// area, so entering edit mode does not replace the field's outline or padding.
fn editor_rect(field: &Field, rect: Rect) -> Rect {
    if field.multiline {
        return Rect {
            x: rect.x + 1.,
            y: rect.y + 1.,
            width: (rect.width - 2.).max(1.),
            height: (rect.height - 2.).max(1.),
        };
    }
    let left = f64::from(field.padding.left);
    let right = f64::from(field.padding.right);
    let height = 22_f64.min(rect.height);
    Rect {
        x: rect.x + left,
        y: rect.y + (rect.height - height) / 2.,
        width: (rect.width - left - right).max(1.),
        height,
    }
}

fn frame_in(view: &NSView, rect: Rect) -> NSRect {
    let bounds = view.bounds();
    let y = if view.isFlipped() {
        rect.y
    } else {
        bounds.size.height - rect.y - rect.height
    };
    NSRect::new(
        NSPoint::new(bounds.origin.x + rect.x, bounds.origin.y + y),
        NSSize::new(rect.width, rect.height),
    )
}

#[component]
pub(crate) fn Mount() -> Element {
    let mut bridge = use_signal(|| None::<Rc<Bridge>>);
    use_future(move || async move {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        let mut editor: Option<Editor> = None;
        loop {
            let handle = bridge.peek().clone();
            if let Some(bridge) = handle {
                if let Some(current) = editor.as_mut() {
                    let action =
                        EDIT.with_borrow(|state| state.as_ref().and_then(|state| state.action));
                    let rect = bridge.rect(current.field.target);
                    if let Some(rect) = rect
                        && action.is_none()
                        && bridge.still_focused(current.field.target)
                    {
                        current.update_position(rect);
                    } else if let Some(previous) = editor.take() {
                        previous.finish(action.unwrap_or("blur"));
                    }
                }
                if editor.is_none()
                    && let Some(field) = bridge.focused()
                    && let Some(window) = app.keyWindow()
                    && window.title().to_string() == "Acheflow"
                {
                    install_edit_menu(&app, mtm);
                    editor = Editor::open(mtm, bridge, window, field);
                }
            }
            crate::scene_preview::pause(16).await;
        }
    });
    rsx! {div {style:"display:none",onmounted:move |event| {
        if let Some(handle) = event.data().downcast::<NodeHandle>() {
            bridge.set(Some(Rc::new(Bridge::new(handle.clone()))));
        }
    }}}
}
