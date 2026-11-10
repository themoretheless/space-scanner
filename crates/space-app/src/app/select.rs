//! Multi-selection: plain click selects one item, ⌘/Ctrl-click adds or takes
//! out, Shift-click (or Shift+arrow) selects the range from the anchor in the
//! order the list shows. `selected` stays the lead item the details pane and
//! single-item actions use; `multi` holds every item while more than one is
//! selected and is empty otherwise.

use std::collections::HashSet;

use space_core::NodeId;

use super::SpaceApp;

/// Pure selection rules, kept apart from the app so they can be tested.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Selection {
    pub lead: Option<NodeId>,
    pub multi: HashSet<NodeId>,
    pub anchor: Option<NodeId>,
}

impl Selection {
    /// Apply a click on `id`. `rows` is the visible order used for ranges.
    pub fn click(&mut self, id: NodeId, toggle: bool, range: bool, rows: &[NodeId]) {
        if range {
            let anchor = self.anchor.or(self.lead).unwrap_or(id);
            let from = rows.iter().position(|&r| r == anchor);
            let to = rows.iter().position(|&r| r == id);
            let mut set = if toggle { self.all() } else { HashSet::new() };
            match (from, to) {
                (Some(a), Some(b)) => set.extend(rows[a.min(b)..=a.max(b)].iter().copied()),
                _ => {
                    set.extend(self.all());
                    set.insert(id);
                }
            }
            self.lead = Some(id);
            self.anchor = Some(anchor);
            self.multi = set;
        } else if toggle {
            let mut set = self.all();
            if !set.remove(&id) {
                set.insert(id);
                self.lead = Some(id);
            } else if self.lead == Some(id) {
                self.lead = set.iter().max_by_key(|i| i.0).copied();
            }
            self.anchor = Some(id);
            self.multi = set;
        } else {
            self.lead = Some(id);
            self.anchor = Some(id);
            self.multi.clear();
        }
        if self.multi.len() <= 1 {
            self.multi.clear();
        }
    }

    /// Every selected id: the multi set, or the lead alone.
    pub fn all(&self) -> HashSet<NodeId> {
        if self.multi.len() > 1 {
            self.multi.clone()
        } else {
            self.lead.into_iter().collect()
        }
    }
}

impl SpaceApp {
    fn selection_state(&self) -> Selection {
        Selection {
            lead: self.selected,
            multi: self.multi.clone(),
            anchor: self.range_anchor,
        }
    }

    fn set_selection_state(&mut self, s: Selection) {
        self.selected = s.lead;
        self.multi = s.multi;
        self.range_anchor = s.anchor;
    }

    /// A click on a row or map item, with the modifiers held.
    pub(super) fn click_select(&mut self, id: NodeId, toggle: bool, range: bool) {
        let mut s = self.selection_state();
        s.click(id, toggle, range, &self.tree_rows);
        self.set_selection_state(s);
    }

    /// Select one item (keyboard, search, details): drops any multi-selection.
    pub(super) fn select_one(&mut self, id: Option<NodeId>) {
        self.selected = id;
        self.multi.clear();
        if id.is_some() {
            self.range_anchor = id;
        }
    }

    /// Every selected item, lead first when there is just one.
    pub(super) fn selection_ids(&self) -> Vec<NodeId> {
        if self.multi.len() > 1 {
            let mut v: Vec<NodeId> = self.multi.iter().copied().collect();
            v.sort_by_key(|i| i.0);
            v
        } else {
            self.selected.into_iter().collect()
        }
    }

    pub(super) fn is_selected(&self, id: NodeId) -> bool {
        self.selected == Some(id) || self.multi.contains(&id)
    }

    /// Keep `multi` consistent with the lead after any direct assignment.
    pub(super) fn sanitize_selection(&mut self) {
        match self.selected {
            Some(lead) if self.multi.contains(&lead) && self.multi.len() > 1 => {}
            _ => self.multi.clear(),
        }
        if let Some(tree) = &self.tree {
            if self.multi.iter().any(|&id| tree.try_node(id).is_none()) {
                self.multi.clear();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[u32]) -> Vec<NodeId> {
        v.iter().map(|&i| NodeId(i)).collect()
    }

    #[test]
    fn plain_click_keeps_single_selection() {
        let rows = ids(&[1, 2, 3, 4]);
        let mut s = Selection::default();
        s.click(NodeId(2), false, false, &rows);
        assert_eq!(s.lead, Some(NodeId(2)));
        assert!(s.multi.is_empty());
        s.click(NodeId(3), false, false, &rows);
        assert_eq!(s.all(), [NodeId(3)].into_iter().collect());
    }

    #[test]
    fn command_click_adds_and_removes() {
        let rows = ids(&[1, 2, 3, 4]);
        let mut s = Selection::default();
        s.click(NodeId(1), false, false, &rows);
        s.click(NodeId(3), true, false, &rows);
        assert_eq!(s.all(), ids(&[1, 3]).into_iter().collect());
        assert_eq!(s.lead, Some(NodeId(3)));
        s.click(NodeId(3), true, false, &rows);
        assert_eq!(s.all(), ids(&[1]).into_iter().collect());
        assert!(s.multi.is_empty());
        assert_eq!(s.lead, Some(NodeId(1)));
    }

    #[test]
    fn shift_click_selects_the_range_in_row_order() {
        let rows = ids(&[5, 1, 9, 2, 7]);
        let mut s = Selection::default();
        s.click(NodeId(1), false, false, &rows);
        s.click(NodeId(7), false, true, &rows);
        assert_eq!(s.all(), ids(&[1, 9, 2, 7]).into_iter().collect());
        // The anchor stays: a second shift-click re-ranges from it.
        s.click(NodeId(5), false, true, &rows);
        assert_eq!(s.all(), ids(&[5, 1]).into_iter().collect());
        // ⌘⇧ adds a range to what is selected.
        s.click(NodeId(2), true, false, &rows);
        s.click(NodeId(7), true, true, &rows);
        assert!(s.all().is_superset(&ids(&[5, 1, 2, 7]).into_iter().collect()));
    }
}
