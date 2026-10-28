use super::{Coverage, Pick, PickReturn, PickView};

/// A picker can visit another photo and return to its working context. Its draft
/// support belongs to that session; the current pointer drag belongs to the visit.
#[derive(Default)]
pub(super) struct Session {
    pub kind: Option<Pick>,
    pub return_to: Option<PickReturn>,
    pub view: Option<PickView>,
    pub region: Option<Coverage>,
}
