//! The accessibility audit: walks the element tree and checks that every widget declares what a
//! screen reader needs (ADR 0009's accessibility baseline).
//!
//! It's split in two so the rules can be tested on their own: [`elements`] reads what each
//! element declares, and [`problems`] checks it.

use std::ops::ControlFlow;

use i_slint_backend_testing::{AccessibleRole, ElementHandle, ElementRoot};

/// Whether the state under audit has tuning access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningAccess {
    Granted,
    /// Without tuning access every control turns read-only in place.
    Denied,
}

/// Chart parts carry no role; the track's readout carries the value.
pub const CHART_PARTS: &[&str] = &["Grid", "Series", "Area", "LimitLine", "CurveLine"];

/// What one element declares to assistive technology.
#[derive(Debug, Clone, Default)]
pub struct Element {
    pub type_name: String,
    pub bases: Vec<String>,
    pub id: String,
    pub role: AccessibleRole,
    pub label: Option<String>,
    pub value: Option<String>,
    pub minimum: Option<f32>,
    pub maximum: Option<f32>,
    pub step: Option<f32>,
    pub checkable: Option<bool>,
    pub checked: Option<bool>,
    pub read_only: Option<bool>,
}

impl Element {
    fn read(handle: &ElementHandle) -> Self {
        Self {
            type_name: handle.type_name().unwrap_or_default().into(),
            bases: handle
                .bases()
                .map(|bases| bases.map(String::from).collect())
                .unwrap_or_default(),
            id: handle.id().unwrap_or_default().into(),
            role: handle.accessible_role().unwrap_or(AccessibleRole::None),
            label: handle.accessible_label().map(String::from),
            value: handle.accessible_value().map(String::from),
            minimum: handle.accessible_value_minimum(),
            maximum: handle.accessible_value_maximum(),
            step: handle.accessible_value_step(),
            checkable: handle.accessible_checkable(),
            checked: handle.accessible_checked(),
            read_only: handle.accessible_read_only(),
        }
    }

    fn is_a(&self, type_name: &str) -> bool {
        self.type_name == type_name || self.bases.iter().any(|base| base == type_name)
    }

    fn label(&self) -> Option<&str> {
        self.label
            .as_deref()
            .filter(|label| !label.trim().is_empty())
    }

    /// The element in a failure: its type name, id, role and label.
    fn describe(&self) -> String {
        let id = if self.id.is_empty() {
            "<no id>"
        } else {
            &self.id
        };
        let label = self
            .label()
            .map(|label| format!(" {label:?}"))
            .unwrap_or_default();
        format!("{} {id} ({:?}{label})", self.type_name, self.role)
    }
}

/// Every visible element below `root`, in tree order.
pub fn elements(root: &impl ElementRoot) -> Vec<Element> {
    let mut elements = Vec::new();
    root.root_element().visit_descendants(|handle| {
        elements.push(Element::read(&handle));
        ControlFlow::<()>::Continue(())
    });
    elements
}

/// What each element fails to declare, one line per problem.
pub fn problems(elements: &[Element], access: TuningAccess) -> Vec<String> {
    use AccessibleRole::*;

    let mut problems = Vec::new();
    for element in elements {
        let mut problem = |what: &str| problems.push(format!("{}: {what}", element.describe()));
        let role = element.role;

        if CHART_PARTS.iter().any(|part| element.is_a(part)) && role != None {
            problem("a chart part with a role; the track's readout carries the value");
        }
        if is_interactive(role) && element.label().is_none() {
            problem("no label");
        }
        if matches!(role, Spinbox | Slider) {
            for (missing, what) in [
                (element.value.is_none(), "no value"),
                (element.minimum.is_none(), "no minimum"),
                (element.maximum.is_none(), "no maximum"),
                (element.step.is_none(), "no step"),
            ] {
                if missing {
                    problem(what);
                }
            }
        }
        if matches!(role, Switch | Checkbox) {
            if element.checkable != Some(true) {
                problem("not checkable");
            }
            if element.checked.is_none() {
                problem("no checked state");
            }
        }
        if access == TuningAccess::Denied && is_editable(role) && element.read_only != Some(true) {
            problem("not read-only");
        }
    }
    problems
}

/// Panics with every problem in the window, naming `state`.
#[track_caller]
pub fn assert_accessible(state: &str, root: &impl ElementRoot, access: TuningAccess) {
    let problems = problems(&elements(root), access);
    assert!(
        problems.is_empty(),
        "{state}: {} accessibility problem(s):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// Roles a user acts on, which a screen reader announces by their label.
pub fn is_interactive(role: AccessibleRole) -> bool {
    use AccessibleRole::*;
    matches!(
        role,
        Button
            | Checkbox
            | Combobox
            | Slider
            | Spinbox
            | Tab
            | TextInput
            | Switch
            | ListItem
            | RadioButton
    )
}

/// Roles of the controls that hold tuning values. A text field isn't one: it searches commands or
/// names things, which needs no tuning access.
fn is_editable(role: AccessibleRole) -> bool {
    use AccessibleRole::*;
    matches!(
        role,
        Checkbox | Combobox | Slider | Spinbox | Switch | RadioButton
    )
}
