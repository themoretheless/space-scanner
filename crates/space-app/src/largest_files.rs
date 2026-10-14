//! Lifecycle of the largest-file query, independent of rendering and thread delivery.

use space_core::{NodeId, SizeMode};

pub type LargestRow = (NodeId, String, u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LargestRequest {
    generation: u64,
    focus: NodeId,
    mode: SizeMode,
    /// Largest folders (subtree totals) rather than files.
    pub dirs: bool,
    /// Rows asked for.
    pub limit: usize,
}

#[derive(Default)]
pub struct LargestFiles {
    generation: u64,
    request: Option<LargestRequest>,
    ready: bool,
    rows: Vec<LargestRow>,
}

impl LargestFiles {
    pub fn invalidate(&mut self) {
        self.request = None;
        self.ready = false;
        self.rows.clear();
    }

    /// Only the first frame for a given query starts work, including while it is running.
    pub fn begin_with(
        &mut self,
        focus: NodeId,
        mode: SizeMode,
        dirs: bool,
        limit: usize,
    ) -> Option<LargestRequest> {
        if self
            .request
            .is_some_and(|r| r.focus == focus && r.mode == mode && r.dirs == dirs && r.limit == limit)
        {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        let request = LargestRequest {
            generation: self.generation,
            focus,
            mode,
            dirs,
            limit,
        };
        self.request = Some(request);
        self.ready = false;
        self.rows.clear();
        Some(request)
    }

    pub fn complete(&mut self, request: LargestRequest, rows: Vec<LargestRow>) {
        // A rescan can reuse node IDs. Comparing focus/mode alone accepts stale results.
        if self.request == Some(request) {
            self.rows = rows;
            self.ready = true;
        }
    }

    pub fn failed(&mut self, request: LargestRequest) {
        if self.request == Some(request) {
            self.invalidate();
        }
    }

    pub fn is_loading(&self) -> bool {
        self.request.is_some() && !self.ready
    }

    /// Changes whenever the rows may have changed (new query or result).
    pub fn version(&self) -> (u64, bool) {
        (self.generation, self.ready)
    }

    pub fn rows(&self) -> &[LargestRow] {
        &self.rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_frames_start_one_query_even_for_an_empty_result() {
        let mut files = LargestFiles::default();
        let request = files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100).unwrap();
        for _ in 0..120 {
            assert_eq!(files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100), None);
        }
        assert!(files.is_loading());
        files.complete(request, vec![]);
        assert!(!files.is_loading());
        assert_eq!(files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100), None);
    }

    #[test]
    fn old_tree_results_do_not_replace_a_rescan_with_the_same_node_ids() {
        let mut files = LargestFiles::default();
        let old = files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100).unwrap();
        files.invalidate();
        let new = files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100).unwrap();
        files.complete(old, vec![(NodeId(999), "old".into(), 100)]);
        assert!(files.is_loading());
        assert!(files.rows().is_empty());
        files.complete(new, vec![(NodeId(1), "new".into(), 10)]);
        files.complete(old, vec![]);
        assert_eq!(files.rows(), &[(NodeId(1), "new".into(), 10)]);
    }

    #[test]
    fn out_of_order_navigation_and_mode_changes_keep_the_latest_query() {
        let mut files = LargestFiles::default();
        let a = files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100).unwrap();
        let b = files.begin_with(NodeId(1), SizeMode::OnDisk, false, 100).unwrap();
        let c = files.begin_with(NodeId(0), SizeMode::Apparent, false, 100).unwrap();
        files.complete(b, vec![]);
        files.failed(a);
        assert!(files.is_loading());
        files.complete(c, vec![]);
        assert!(!files.is_loading());
        assert_eq!(files.begin_with(NodeId(0), SizeMode::Apparent, false, 100), None);
    }

    #[test]
    fn switching_to_folders_or_another_limit_starts_a_new_query() {
        let mut files = LargestFiles::default();
        let a = files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100).unwrap();
        assert_eq!(files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100), None);
        let b = files.begin_with(NodeId(0), SizeMode::OnDisk, true, 100).unwrap();
        assert!(b.dirs && a != b);
        let c = files.begin_with(NodeId(0), SizeMode::OnDisk, true, 50).unwrap();
        assert_eq!(c.limit, 50);
        files.complete(a, vec![(NodeId(1), "stale".into(), 1)]);
        assert!(files.rows().is_empty());
    }

    #[test]
    fn failed_spawn_can_be_retried_and_invalidated_results_are_ignored() {
        let mut files = LargestFiles::default();
        let old = files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100).unwrap();
        files.failed(old);
        assert!(!files.is_loading());
        let retry = files.begin_with(NodeId(0), SizeMode::OnDisk, false, 100).unwrap();
        assert_ne!(old, retry);
        files.invalidate();
        files.complete(retry, vec![(NodeId(1), "late".into(), 1)]);
        assert!(files.rows().is_empty());
        assert!(!files.is_loading());
    }
}
