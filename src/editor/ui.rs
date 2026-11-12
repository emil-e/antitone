//! Shared egui components and visual conventions. Features compose them and own application state.

mod actions;
pub(super) use actions::TextAction;

mod application;
pub(super) use application::{AppPanel, app_header, app_panel};

mod balance;
pub(super) use balance::{BalanceControl, ColorControl, balance_pick_tool};

mod cards;
pub(super) use cards::ContactSheetLayout;

mod dialogs;
pub(super) use dialogs::{DecisionDialog, DialogChoice, escape_requested, focus_default, modal};

mod feedback;
pub(super) use feedback::{ContextualStatus, FeedbackDialog, Severity, StatusLevel, status_icon};

pub(super) mod icons;
pub(super) use icons::Icon;

mod inspector;
pub(super) use inspector::{
    INSPECTOR_ROW_HEIGHT, INSPECTOR_SECTION_GAP, PARAMETER_GAP, PARAMETER_NUMERIC_WIDTH,
    PARAMETER_SLIDER_HEIGHT, PARAMETER_UNIT_WIDTH, SectionHeader, SectionTool, collect_expansion,
    draggable_icon_row, inspector_dropdown, inspector_row, inspector_spacing,
    labelled_inspector_dropdown, labelled_row, labelled_row_at, name_label, restore_expansion,
    section, section_disclosure, section_with_secondary_tool, stage_tabs,
};

mod interaction;
pub(super) use interaction::{ControlResponse, numeric_cancelled, values_are_being_adjusted};

mod parameter;
pub(super) use parameter::ParameterRow;

mod progress;
pub(super) use progress::{ACTIVITY_BAR_HEIGHT, Progress, ProgressContent};

mod theme;
pub(super) use theme::{
    CONTROL_HEIGHT, configure_branding, configure_fonts, instrument_label, instrument_readout,
};

mod tools;
pub(super) use tools::{ToolButton, ToolGroup, ToolIdentity, ToolShelf};

mod viewer;
pub(super) use viewer::{VIEWER_ITEM_GAP, ViewerBar, viewer_surface};

#[cfg(test)]
pub(in crate::editor) mod test_support;

#[cfg(test)]
mod tests;
