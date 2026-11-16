use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use eframe::egui::Color32;
use space_core::{NodeId, SizeMode, SpaceTree};

use crate::downloads::DownloadsState;
use crate::junk_panel::JunkPanelState;
use crate::largest_files::{InsightTab, LargestFiles};
use crate::review;
use crate::theme::{self, MUTED};
use crate::views::{self, ViewFilter};

use super::{basket_total_bytes, is_live_node, BasketItem};

/// Hard cap on drawn rows: an opened `node_modules` must not stall a frame.
pub(super) const MAX_TREE_ROWS: usize = 2_000;
/// Children shown per folder before the rest fold into "N more".
const MAX_KIDS: usize = 200;

pub(super) struct TreeRow {
    pub(super) id: NodeId,
    pub(super) depth: usize,
    pub(super) color: Color32,
    pub(super) size: u64,
    /// Share of the parent folder, 0..=1.
    pub(super) share: f32,
    pub(super) has_kids: bool,
    pub(super) open: bool,
    /// For each ancestor level: does a later sibling follow (draw a guide)?
    pub(super) guides: Vec<bool>,
}

pub(super) struct ContentsListing {
    pub(super) version: u64,
    pub(super) ids: Vec<NodeId>,
    pub(super) before: usize,
    pub(super) rows: Vec<TreeRow>,
    pub(super) row_ids: Vec<NodeId>,
}

pub(super) struct ReviewSets {
    pub(super) queued: HashSet<NodeId>,
    pub(super) partial: HashSet<NodeId>,
    pub(super) bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct BasketKey {
    tree_ptr: usize,
    tree_epoch: u64,
    size_mode: SizeMode,
    review_sig: u64,
    junk_ver: u64,
    downloads_ver: u64,
}

struct BasketEntry {
    key: BasketKey,
    items: Vec<BasketItem>,
    total_bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ReviewKey {
    tree_ptr: usize,
    tree_epoch: u64,
    size_mode: SizeMode,
    review_sig: u64,
}

struct ReviewEntry {
    key: ReviewKey,
    sets: Rc<ReviewSets>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ContentsKey {
    tree_ptr: usize,
    tree_epoch: u64,
    focus: NodeId,
    size_mode: SizeMode,
    filter_hash: u64,
    flat: bool,
    show_largest: bool,
    insight_tab: Option<InsightTab>,
    largest_gen: u64,
    largest_ready: bool,
    expanded_sig: u64,
    theme_bits: u64,
}

struct ContentsEntry {
    key: ContentsKey,
    listing: Rc<ContentsListing>,
}

/// Per-frame memoized derivations for the UI thread, keyed by state versions
/// so unchanged frames perform zero cloning, sorting, or tree walking.
#[derive(Default)]
pub(super) struct AppCaches {
    tree_epoch: Cell<u64>,
    listing_seq: Cell<u64>,
    synced_listing_ver: Cell<u64>,
    basket: RefCell<Option<Rc<BasketEntry>>>,
    review: RefCell<Option<ReviewEntry>>,
    contents: RefCell<Option<ContentsEntry>>,
}

impl AppCaches {
    pub(super) fn bump_tree_epoch(&self) {
        self.tree_epoch.set(self.tree_epoch.get().wrapping_add(1));
        self.synced_listing_ver.set(0);
    }

    pub(super) fn invalidate_all(&self) {
        self.bump_tree_epoch();
        *self.basket.borrow_mut() = None;
        *self.review.borrow_mut() = None;
        *self.contents.borrow_mut() = None;
    }

    fn basket_entry(
        &self,
        tree: Option<&Arc<SpaceTree>>,
        mode: SizeMode,
        review: &[NodeId],
        junk: &JunkPanelState,
        downloads: &DownloadsState,
    ) -> Rc<BasketEntry> {
        let key = BasketKey {
            tree_ptr: tree.map_or(0, |t| Arc::as_ptr(t) as usize),
            tree_epoch: self.tree_epoch.get(),
            size_mode: mode,
            review_sig: review::signature(review),
            junk_ver: junk.selection_version(),
            downloads_ver: downloads.selection_version(),
        };
        if let Some(entry) = self.basket.borrow().as_ref() {
            if entry.key == key {
                return Rc::clone(entry);
            }
        }
        let items = compute_basket_items(tree.map(|t| t.as_ref()), mode, review, junk, downloads);
        let total_bytes = basket_total_bytes(&items);
        let entry = Rc::new(BasketEntry {
            key,
            items,
            total_bytes,
        });
        *self.basket.borrow_mut() = Some(Rc::clone(&entry));
        entry
    }

    pub(super) fn basket_items(
        &self,
        tree: Option<&Arc<SpaceTree>>,
        mode: SizeMode,
        review: &[NodeId],
        junk: &JunkPanelState,
        downloads: &DownloadsState,
    ) -> Vec<BasketItem> {
        self.basket_entry(tree, mode, review, junk, downloads)
            .items
            .clone()
    }

    pub(super) fn basket_len(
        &self,
        tree: Option<&Arc<SpaceTree>>,
        mode: SizeMode,
        review: &[NodeId],
        junk: &JunkPanelState,
        downloads: &DownloadsState,
    ) -> usize {
        self.basket_entry(tree, mode, review, junk, downloads)
            .items
            .len()
    }

    pub(super) fn basket_bytes(
        &self,
        tree: Option<&Arc<SpaceTree>>,
        mode: SizeMode,
        review: &[NodeId],
        junk: &JunkPanelState,
        downloads: &DownloadsState,
    ) -> u64 {
        self.basket_entry(tree, mode, review, junk, downloads)
            .total_bytes
    }

    pub(super) fn review_sets(
        &self,
        tree: &Arc<SpaceTree>,
        review: &[NodeId],
        mode: SizeMode,
    ) -> Rc<ReviewSets> {
        let key = ReviewKey {
            tree_ptr: Arc::as_ptr(tree) as usize,
            tree_epoch: self.tree_epoch.get(),
            size_mode: mode,
            review_sig: review::signature(review),
        };
        if let Some(entry) = self.review.borrow().as_ref() {
            if entry.key == key {
                return Rc::clone(&entry.sets);
            }
        }
        let queued: HashSet<NodeId> = review
            .iter()
            .copied()
            .filter(|&id| tree.try_node(id).is_some())
            .collect();
        let mut partial = HashSet::new();
        for &qid in &queued {
            let mut parent = tree.try_node(qid).and_then(|n| n.parent);
            while let Some(pid) = parent {
                if !partial.insert(pid) {
                    // Ancestor chain from this point up was already inserted.
                    break;
                }
                parent = tree.try_node(pid).and_then(|n| n.parent);
            }
        }
        let bytes = review::queue_bytes(tree, review, mode);
        let sets = Rc::new(ReviewSets {
            queued,
            partial,
            bytes,
        });
        *self.review.borrow_mut() = Some(ReviewEntry {
            key,
            sets: Rc::clone(&sets),
        });
        sets
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn contents_listing(
        &self,
        tree: &Arc<SpaceTree>,
        focus: NodeId,
        mode: SizeMode,
        filter: &ViewFilter,
        expanded: &HashSet<NodeId>,
        flat: bool,
        show_largest: bool,
        insight_tab: InsightTab,
        largest: &LargestFiles,
    ) -> Rc<ContentsListing> {
        let (largest_gen, largest_ready) = if show_largest {
            largest.state_token()
        } else {
            (0, false)
        };
        let key = ContentsKey {
            tree_ptr: Arc::as_ptr(tree) as usize,
            tree_epoch: self.tree_epoch.get(),
            focus,
            size_mode: mode,
            filter_hash: filter.hash64(),
            flat,
            show_largest,
            insight_tab: show_largest.then_some(insight_tab),
            largest_gen,
            largest_ready,
            expanded_sig: if flat { 0 } else { expanded_sig(expanded) },
            theme_bits: views::theme_bits(),
        };
        if let Some(entry) = self.contents.borrow().as_ref() {
            if entry.key == key {
                return Rc::clone(&entry.listing);
            }
        }
        let next_ver = self.listing_seq.get().wrapping_add(1).max(1);
        self.listing_seq.set(next_ver);
        let listing = Rc::new(build_contents_listing(
            next_ver,
            tree,
            focus,
            mode,
            filter,
            expanded,
            flat,
            show_largest,
            insight_tab,
            largest,
        ));
        *self.contents.borrow_mut() = Some(ContentsEntry {
            key,
            listing: Rc::clone(&listing),
        });
        listing
    }

    pub(super) fn sync_tree_rows(&self, listing: &ContentsListing, tree_rows: &mut Vec<NodeId>) {
        if self.synced_listing_ver.get() != listing.version
            || tree_rows.len() != listing.row_ids.len()
        {
            tree_rows.clone_from(&listing.row_ids);
            self.synced_listing_ver.set(listing.version);
        }
    }
}

#[inline]
fn mix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

pub(super) fn expanded_sig(expanded: &HashSet<NodeId>) -> u64 {
    let mut xor_acc = 0_u64;
    let mut sum_acc = 0_u64;
    for &id in expanded {
        let z = mix64(id.0 as u64);
        xor_acc ^= z;
        sum_acc = sum_acc.wrapping_add(z);
    }
    let mixed = mix64(expanded.len() as u64) ^ mix64(xor_acc.rotate_left(17) ^ sum_acc);
    mix64(mixed)
}

fn compute_basket_items(
    tree: Option<&SpaceTree>,
    size_mode: SizeMode,
    review: &[NodeId],
    junk: &JunkPanelState,
    downloads: &DownloadsState,
) -> Vec<BasketItem> {
    let mut items = Vec::new();
    if let Some(tree) = tree {
        items.extend(
            review
                .iter()
                .copied()
                .filter(|&id| {
                    is_live_node(tree, id)
                        && tree
                            .try_node(id)
                            .is_some_and(|n| !n.synthetic && n.parent.is_some())
                })
                .filter_map(|id| {
                    let node = tree.try_node(id)?;
                    Some(BasketItem {
                        path: PathBuf::from(tree.path_string(id)),
                        bytes: node.size(size_mode),
                        is_dir: node.is_dir,
                        junk_guard: None,
                    })
                }),
        );
    }
    // Use the cached finding.is_dir flag here instead of per-frame lstat syscalls;
    // deletion workers revalidate paths on the background thread right before acting.
    items.extend(
        junk.selected_findings()
            .into_iter()
            .map(|finding| BasketItem {
                path: finding.path,
                bytes: finding.bytes,
                is_dir: finding.is_dir,
                junk_guard: Some((finding.root, finding.min_age_days)),
            }),
    );
    items.extend(
        downloads
            .selected_items()
            .into_iter()
            .map(|item| BasketItem {
                path: item.path,
                bytes: item.bytes,
                is_dir: item.is_dir,
                junk_guard: None,
            }),
    );
    let mut coalesced = space_clean::coalesce_nested_paths(items, |item| item.path.as_path());
    coalesced.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    coalesced
}

#[allow(clippy::too_many_arguments)]
fn build_contents_listing(
    version: u64,
    tree: &SpaceTree,
    focus: NodeId,
    mode: SizeMode,
    filter: &ViewFilter,
    expanded: &HashSet<NodeId>,
    flat: bool,
    show_largest: bool,
    insight_tab: InsightTab,
    largest: &LargestFiles,
) -> ContentsListing {
    let mut ids: Vec<NodeId> = if show_largest {
        let data = largest.data();
        match insight_tab {
            InsightTab::Files => data.files.iter().map(|row| row.0).collect(),
            InsightTab::Folders => data.folders.iter().map(|row| row.0).collect(),
            InsightTab::Oldest => data.oldest.iter().map(|row| row.0).collect(),
            InsightTab::Duplicates => data
                .duplicates
                .iter()
                .flat_map(|(_, _, nodes)| nodes.iter().copied())
                .collect(),
            InsightTab::Types => Vec::new(),
        }
    } else {
        tree.children(focus).to_vec()
    };
    let before = ids.len();
    ids.retain(|&c| {
        let Some(n) = tree.try_node(c) else {
            return false;
        };
        filter.matches(&n.name, n.is_dir, n.size(mode))
    });
    if !show_largest || insight_tab != InsightTab::Oldest {
        ids.sort_by_key(|&c| std::cmp::Reverse(tree.node(c).size(mode)));
    }
    let items: Vec<(&str, bool)> = ids
        .iter()
        .map(|&c| {
            let n = tree.node(c);
            (n.name.as_str(), n.is_dir)
        })
        .collect();
    let colors = theme::sibling_palette(&items);
    let rows = tree_rows(tree, &ids, &colors, expanded, flat, mode);
    let row_ids = rows.iter().map(|r| r.id).collect();
    ContentsListing {
        version,
        ids,
        before,
        rows,
        row_ids,
    }
}

pub(super) fn tree_rows(
    tree: &SpaceTree,
    top: &[NodeId],
    colors: &[Color32],
    expanded: &HashSet<NodeId>,
    flat: bool,
    mode: SizeMode,
) -> Vec<TreeRow> {
    struct WalkCtx<'a> {
        tree: &'a SpaceTree,
        expanded: &'a HashSet<NodeId>,
        mode: SizeMode,
        parent_total: &'a dyn Fn(NodeId) -> f32,
        out: Vec<TreeRow>,
    }

    fn walk(ctx: &mut WalkCtx<'_>, id: NodeId, depth: usize, color: Color32, guides: Vec<bool>) {
        if ctx.out.len() >= MAX_TREE_ROWS {
            return;
        }
        let n = ctx.tree.node(id);
        let kids = if n.is_dir && !n.synthetic {
            ctx.tree.children(id)
        } else {
            &[]
        };
        let open = !kids.is_empty() && ctx.expanded.contains(&id);
        ctx.out.push(TreeRow {
            id,
            depth,
            color,
            size: n.size(ctx.mode),
            share: n.size(ctx.mode) as f32 / (ctx.parent_total)(id),
            has_kids: !kids.is_empty(),
            open,
            guides: guides.clone(),
        });
        if open {
            let mut kids = kids.to_vec();
            kids.sort_by_key(|&c| std::cmp::Reverse(ctx.tree.node(c).size(ctx.mode)));
            kids.truncate(MAX_KIDS);
            let last = kids.len().saturating_sub(1);
            for (i, k) in kids.into_iter().enumerate() {
                let mut g = guides.clone();
                g.push(i != last);
                let c = if ctx.tree.node(k).synthetic {
                    MUTED()
                } else {
                    color
                };
                walk(ctx, k, depth + 1, c, g);
            }
        }
    }

    let parent_total = |id: NodeId| {
        tree.node(id)
            .parent
            .map(|p| tree.node(p).size(mode))
            .unwrap_or(0)
            .max(1) as f32
    };
    let mut ctx = WalkCtx {
        tree,
        expanded,
        mode,
        parent_total: &parent_total,
        out: Vec::new(),
    };
    for (i, &id) in top.iter().enumerate() {
        let color = if tree.node(id).synthetic {
            MUTED()
        } else {
            colors[i]
        };
        if flat {
            ctx.out.push(TreeRow {
                id,
                depth: 0,
                color,
                size: tree.node(id).size(mode),
                share: tree.node(id).size(mode) as f32 / parent_total(id),
                has_kids: false,
                open: false,
                guides: Vec::new(),
            });
        } else {
            walk(&mut ctx, id, 0, color, Vec::new());
        }
        if ctx.out.len() >= MAX_TREE_ROWS {
            break;
        }
    }
    ctx.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloads::{DownloadItem, DownloadKind};
    use space_core::Node;

    fn sample_tree() -> Arc<SpaceTree> {
        Arc::new(SpaceTree {
            nodes: vec![
                Node {
                    name: "root".into(),
                    apparent_size: 300,
                    allocated_size: 300,
                    file_count: 2,
                    is_dir: true,
                    synthetic: false,
                    mtime_unix: 1,
                    children: vec![NodeId(1)],
                    parent: None,
                },
                Node {
                    name: "sub".into(),
                    apparent_size: 300,
                    allocated_size: 300,
                    file_count: 2,
                    is_dir: true,
                    synthetic: false,
                    mtime_unix: 1,
                    children: vec![NodeId(2), NodeId(3)],
                    parent: Some(NodeId(0)),
                },
                Node {
                    name: "a.bin".into(),
                    apparent_size: 100,
                    allocated_size: 100,
                    file_count: 1,
                    is_dir: false,
                    synthetic: false,
                    mtime_unix: 1,
                    children: vec![],
                    parent: Some(NodeId(1)),
                },
                Node {
                    name: "b.bin".into(),
                    apparent_size: 200,
                    allocated_size: 200,
                    file_count: 1,
                    is_dir: false,
                    synthetic: false,
                    mtime_unix: 1,
                    children: vec![],
                    parent: Some(NodeId(1)),
                },
            ],
            root: NodeId(0),
            root_path: "/tmp/sample".into(),
            scanned_at_unix: 1,
            item_count: 4,
            skipped_errors: 0,
            hardlink_candidates: 0,
            hardlinks_deduped: 0,
            skipped_roots: Vec::new(),
        })
    }

    #[test]
    fn expanded_sig_is_order_independent() {
        let mut a = HashSet::new();
        for n in [1_u32, 7, 42, 99] {
            a.insert(NodeId(n));
        }
        let mut b = HashSet::new();
        for n in [99_u32, 42, 1, 7] {
            b.insert(NodeId(n));
        }
        assert_eq!(expanded_sig(&a), expanded_sig(&b));
        b.remove(&NodeId(42));
        assert_ne!(expanded_sig(&a), expanded_sig(&b));
    }

    #[test]
    fn basket_cache_reuses_and_invalidates_on_selection_change() {
        let tree = sample_tree();
        let caches = AppCaches::default();
        let junk = JunkPanelState::default();
        let mut downloads = DownloadsState::default();
        downloads.set_items(vec![DownloadItem {
            path: PathBuf::from("/tmp/sample/dl.dmg"),
            name: "dl.dmg".into(),
            bytes: 50,
            is_dir: false,
            kind: DownloadKind::Installer,
            age_secs: None,
        }]);
        let mut review = vec![NodeId(2)];

        let p1 = Rc::as_ptr(&caches.basket_entry(
            Some(&tree),
            SizeMode::OnDisk,
            &review,
            &junk,
            &downloads,
        ));
        let p2 = Rc::as_ptr(&caches.basket_entry(
            Some(&tree),
            SizeMode::OnDisk,
            &review,
            &junk,
            &downloads,
        ));
        assert_eq!(p1, p2, "unchanged inputs should reuse cached Rc");
        assert_eq!(
            caches.basket_bytes(Some(&tree), SizeMode::OnDisk, &review, &junk, &downloads),
            100
        );

        downloads.set_selected(0, true);
        let p3 = Rc::as_ptr(&caches.basket_entry(
            Some(&tree),
            SizeMode::OnDisk,
            &review,
            &junk,
            &downloads,
        ));
        assert_ne!(p1, p3);
        assert_eq!(
            caches.basket_bytes(Some(&tree), SizeMode::OnDisk, &review, &junk, &downloads),
            150
        );

        review.push(NodeId(3));
        assert_eq!(
            caches.basket_bytes(Some(&tree), SizeMode::OnDisk, &review, &junk, &downloads),
            350
        );
    }

    #[test]
    fn review_set_cache_matches_queue_bytes() {
        let tree = sample_tree();
        let caches = AppCaches::default();
        let review = vec![NodeId(2)];
        let sets1 = caches.review_sets(&tree, &review, SizeMode::OnDisk);
        let sets2 = caches.review_sets(&tree, &review, SizeMode::OnDisk);
        assert_eq!(Rc::as_ptr(&sets1), Rc::as_ptr(&sets2));
        assert!(sets1.queued.contains(&NodeId(2)));
        assert!(sets1.partial.contains(&NodeId(1)));
        assert!(sets1.partial.contains(&NodeId(0)));
        assert_eq!(
            sets1.bytes,
            review::queue_bytes(&tree, &review, SizeMode::OnDisk)
        );
    }
}
