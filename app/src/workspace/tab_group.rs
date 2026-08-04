//! Tab group data model. Gated at runtime by `FeatureFlag::GroupedTabs`.

use uuid::Uuid;
use warpui::elements::DraggableState;

use crate::tab::SelectedTabColor;

/// Header label shown for a group the user hasn't named yet.
pub const DEFAULT_TAB_GROUP_NAME: &str = "New Group";

/// Stable identity for a tab group.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TabGroupId(pub Uuid);

impl TabGroupId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for TabGroupId {
    fn default() -> Self {
        Self::new()
    }
}

/// A named group of tabs in the vertical tabs panel.
/// Member tabs reference their group via `TabData::group_id`.
#[derive(Clone)]
pub struct TabGroup {
    pub id: TabGroupId,
    pub name: Option<String>,
    pub color: SelectedTabColor,
    pub collapsed: bool,
    pub draggable_state: DraggableState,
    /// True when this whole group is pinned to the front of the tab list.
    pub pinned: bool,
}

impl TabGroup {
    /// The group's name as displayed in its header.
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(DEFAULT_TAB_GROUP_NAME)
    }

    /// Creates a new, untitled, expanded tab group with a fresh id.
    pub fn new() -> Self {
        Self {
            id: TabGroupId::new(),
            name: None,
            color: SelectedTabColor::default(),
            collapsed: false,
            draggable_state: Default::default(),
            pinned: false,
        }
    }
}

impl Default for TabGroup {
    fn default() -> Self {
        Self::new()
    }
}
