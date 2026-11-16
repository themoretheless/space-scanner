//! Lifecycle of the largest-file & insights query, independent of rendering and thread delivery.

use space_core::{NodeId, SizeMode};

pub type LargestRow = (NodeId, String, u64);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InsightTab {
    #[default]
    Files,
    Folders,
    Oldest,
    Duplicates,
    Types,
}

impl InsightTab {
    pub const ALL: [Self; 5] = [
        Self::Files,
        Self::Folders,
        Self::Oldest,
        Self::Duplicates,
        Self::Types,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Folders => "Folders",
            Self::Oldest => "Oldest",
            Self::Duplicates => "Duplicates",
            Self::Types => "Types",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InsightData {
    pub files: Vec<LargestRow>,
    pub folders: Vec<LargestRow>,
    pub oldest: Vec<(NodeId, String, u64, u64)>,
    pub duplicates: Vec<(String, u64, Vec<NodeId>)>,
    pub types: Vec<(String, u64, u64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LargestRequest {
    generation: u64,
    focus: NodeId,
    mode: SizeMode,
}

#[derive(Default)]
pub struct LargestFiles {
    generation: u64,
    request: Option<LargestRequest>,
    ready: bool,
    data: InsightData,
}

impl LargestFiles {
    pub fn invalidate(&mut self) {
        self.request = None;
        self.ready = false;
        self.data = InsightData::default();
    }

    pub fn state_token(&self) -> (u64, bool) {
        (self.generation, self.ready)
    }

    /// Only the first frame for a given query starts work, including while it is running.
    pub fn begin(&mut self, focus: NodeId, mode: SizeMode) -> Option<LargestRequest> {
        if self
            .request
            .is_some_and(|r| r.focus == focus && r.mode == mode)
        {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        let request = LargestRequest {
            generation: self.generation,
            focus,
            mode,
        };
        self.request = Some(request);
        self.ready = false;
        self.data = InsightData::default();
        Some(request)
    }

    #[cfg(test)]
    pub fn complete(&mut self, request: LargestRequest, rows: Vec<LargestRow>) {
        self.complete_data(
            request,
            InsightData {
                files: rows,
                ..InsightData::default()
            },
        );
    }

    pub fn complete_data(&mut self, request: LargestRequest, data: InsightData) {
        // A rescan can reuse node IDs. Comparing focus/mode alone accepts stale results.
        if self.request == Some(request) {
            self.data = data;
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

    #[cfg(test)]
    pub fn rows(&self) -> &[LargestRow] {
        &self.data.files
    }

    pub fn data(&self) -> &InsightData {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_frames_start_one_query_even_for_an_empty_result() {
        let mut files = LargestFiles::default();
        let request = files.begin(NodeId(0), SizeMode::OnDisk).unwrap();
        for _ in 0..120 {
            assert_eq!(files.begin(NodeId(0), SizeMode::OnDisk), None);
        }
        assert!(files.is_loading());
        files.complete(request, vec![]);
        assert!(!files.is_loading());
        assert_eq!(files.begin(NodeId(0), SizeMode::OnDisk), None);
    }

    #[test]
    fn old_tree_results_do_not_replace_a_rescan_with_the_same_node_ids() {
        let mut files = LargestFiles::default();
        let old = files.begin(NodeId(0), SizeMode::OnDisk).unwrap();
        files.invalidate();
        let new = files.begin(NodeId(0), SizeMode::OnDisk).unwrap();
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
        let a = files.begin(NodeId(0), SizeMode::OnDisk).unwrap();
        let b = files.begin(NodeId(1), SizeMode::OnDisk).unwrap();
        let c = files.begin(NodeId(0), SizeMode::Apparent).unwrap();
        files.complete(b, vec![]);
        files.failed(a);
        assert!(files.is_loading());
        files.complete(c, vec![]);
        assert!(!files.is_loading());
        assert_eq!(files.begin(NodeId(0), SizeMode::Apparent), None);
    }

    #[test]
    fn failed_spawn_can_be_retried_and_invalidated_results_are_ignored() {
        let mut files = LargestFiles::default();
        let old = files.begin(NodeId(0), SizeMode::OnDisk).unwrap();
        files.failed(old);
        assert!(!files.is_loading());
        let retry = files.begin(NodeId(0), SizeMode::OnDisk).unwrap();
        assert_ne!(old, retry);
        files.invalidate();
        files.complete(retry, vec![(NodeId(1), "late".into(), 1)]);
        assert!(files.rows().is_empty());
        assert!(!files.is_loading());
    }

    #[test]
    fn insight_data_tracks_all_tabs_for_active_generation() {
        let mut files = LargestFiles::default();
        let r1 = files.begin(NodeId(0), SizeMode::OnDisk).unwrap();
        let data = InsightData {
            files: vec![(NodeId(1), "big.bin".into(), 500)],
            folders: vec![(NodeId(2), "build".into(), 1000)],
            oldest: vec![(NodeId(3), "old.zip".into(), 200, 1_600_000_000)],
            duplicates: vec![("dup.dmg".into(), 300, vec![NodeId(4), NodeId(5)])],
            types: vec![("dmg".into(), 300, 2)],
        };
        files.complete_data(r1, data.clone());
        assert_eq!(files.data(), &data);
    }
}
