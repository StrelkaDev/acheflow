use dioxus_native::prelude::*;

#[derive(Clone, Debug, PartialEq)]
struct FpsDraft {
    text: String,
    error: Option<String>,
}

impl FpsDraft {
    fn new(value: u32) -> Self {
        Self {
            text: value.to_string(),
            error: None,
        }
    }

    fn input(&mut self, text: String) {
        self.text = text;
        self.error = None;
    }

    fn commit(&mut self, current: u32) -> Option<u32> {
        match self.text.trim().parse::<u32>() {
            Ok(value) if (1..=120).contains(&value) => {
                *self = Self::new(value);
                Some(value)
            }
            _ => {
                self.text = current.to_string();
                self.error = Some(format!(
                    "Enter a whole number from 1 to 120. Kept {current} FPS."
                ));
                None
            }
        }
    }
}

#[component]
pub fn FrameRateInput(value: u32, change: EventHandler<u32>) -> Element {
    let mut draft = use_signal(|| FpsDraft::new(value));
    rsx! {div {class:"field",
        label {"Frame rate (FPS)",input {
            r#type:"text",inputmode:"numeric","aria-label":"Frame rate (FPS)",
            "aria-invalid":draft.read().error.is_some().to_string(),
            value:draft.read().text.clone(),
            oninput:move |e|draft.write().input(e.value()),
            onblur:move |_| {let next=draft.write().commit(value);if let Some(fps)=next {change.call(fps);}},
            onkeydown:move |e| {
                if e.key()==Key::Named(NamedKey::Enter) {
                    e.prevent_default();
                    let next=draft.write().commit(value);if let Some(fps)=next {change.call(fps);}
                } else if e.key()==Key::Named(NamedKey::Escape) {draft.set(FpsDraft::new(value));}
            },
        }}
        div {class:"row fps-choices",role:"group","aria-label":"Common frame rates",
            for fps in [24,25,30,50,60,120] {button {class:if value==fps {"selected"}else{""},"aria-pressed":(value==fps).to_string(),onclick:move |_| {draft.set(FpsDraft::new(fps));change.call(fps);},"{fps}"}}
        }
        span {class:"form-help","Applied: {value} FPS · Enter or leave the field to apply"}
        if let Some(error)=draft.read().error.as_ref() {span {class:"field-error",role:"alert","{error}"}}
    }}
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_and_invalid_edits_restore_the_applied_fps() {
        let mut draft = FpsDraft::new(60);
        for text in ["", "0", "600", "-1", "29.97", "abc", "4294967296"] {
            draft.input(text.into());
            assert_eq!(draft.commit(60), None);
            assert_eq!(draft.text, "60");
            assert!(draft.error.is_some());
        }
    }
    #[test]
    fn multi_digit_edits_are_committed_together() {
        let mut draft = FpsDraft::new(30);
        draft.input(String::new());
        draft.input("6".into());
        draft.input("60".into());
        assert_eq!(draft.commit(30), Some(60));
        assert_eq!(draft.text, "60");
        assert!(draft.error.is_none());
        for fps in [1, 24, 25, 30, 50, 60, 120] {
            draft.input(format!(" {fps} "));
            assert_eq!(draft.commit(30), Some(fps));
        }
    }
}
