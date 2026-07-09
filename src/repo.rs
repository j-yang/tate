//! Version control kernel for structured data.
//!
//! A [`Repo`] stores versions (as [`Section`](crate::section::Section)s) in a
//! content-addressed map and tracks their lineage in a commit DAG. Merge is
//! computed via [`patch::merge_sections`] — the pushout of the span
//! `ours <- base -> theirs`.
//!
//! ```
//! use tate::tree::TreeNode;
//! use tate::repo::Repo;
//!
//! let mut repo = Repo::new();
//!
//! // Commit an initial version with two nodes.
//! let v0 = repo.commit("initial", &[], &TreeNode::new("root")
//!     .with_child(TreeNode::new("server").with_identity("s1")
//!         .with_attr("port", "8080"))
//!     .with_child(TreeNode::new("db").with_identity("d1")
//!         .with_attr("url", "localhost")));
//!
//! // Branch A: change server.
//! let v1 = repo.commit("port -> 9090", &[v0], &TreeNode::new("root")
//!     .with_child(TreeNode::new("server").with_identity("s1")
//!         .with_attr("port", "9090"))
//!     .with_child(TreeNode::new("db").with_identity("d1")
//!         .with_attr("url", "localhost")));
//!
//! // Branch B: change db (disjoint node).
//! let v2 = repo.commit("url -> prod", &[v0], &TreeNode::new("root")
//!     .with_child(TreeNode::new("server").with_identity("s1")
//!         .with_attr("port", "8080"))
//!     .with_child(TreeNode::new("db").with_identity("d1")
//!         .with_attr("url", "prod.example.com")));
//!
//! // Merge: disjoint nodes -> clean pushout.
//! let result = repo.merge(v1, v2);
//! assert!(result.is_clean());
//! ```

use crate::patch::{self, merge_sections, ApplyError, Patch, SectionConflict};
use crate::section::Section;
use crate::tree::TreeNode;
use std::collections::{HashMap, HashSet};

/// Content-addressed hash (FNV-1a, deterministic across Rust versions).
pub type Hash = u64;

/// A commit in the version DAG.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Commit {
    /// Parent commit hashes (0 = initial, 1 = linear, 2 = merge).
    pub parents: Vec<Hash>,
    /// Content hash of the [`Section`] this commit points to.
    pub version: Hash,
    /// Human-readable message.
    pub message: String,
    /// Unix timestamp (seconds).
    pub timestamp: u64,
}

/// A version control repository: content-addressed sections + commit DAG + branches.
pub struct Repo {
    sections: HashMap<Hash, Section>,
    commits: HashMap<Hash, Commit>,
    branches: HashMap<String, Hash>,
    head: Option<Hash>,
}

/// Result of a merge operation.
#[derive(Debug)]
pub struct MergeResult {
    /// Content hash of the merged [`Section`].
    pub merged_section: Hash,
    /// Conflict list (empty if clean).
    pub conflicts: Vec<SectionConflict>,
    /// The merge base used.
    pub base: Hash,
    pub ours: Hash,
    pub theirs: Hash,
}

impl MergeResult {
    /// True when the merge produced no conflicts.
    pub fn is_clean(&self) -> bool {
        self.conflicts.is_empty()
    }
}

impl Repo {
    /// Create an empty repository.
    pub fn new() -> Self {
        Repo {
            sections: HashMap::new(),
            commits: HashMap::new(),
            branches: HashMap::new(),
            head: None,
        }
    }

    fn store_section(&mut self, section: Section) -> Hash {
        let h = hash_section(&section);
        self.sections.entry(h).or_insert(section);
        h
    }

    /// Commit a version from a [`TreeNode`]. Returns the commit hash.
    pub fn commit(&mut self, message: &str, parents: &[Hash], tree: &TreeNode) -> Hash {
        self.commit_section(message, parents, tree.to_section())
    }

    /// Commit a version from a [`Section`]. Returns the commit hash.
    pub fn commit_section(
        &mut self,
        message: &str,
        parents: &[Hash],
        section: Section,
    ) -> Hash {
        let version = self.store_section(section);
        let commit = Commit {
            parents: parents.to_vec(),
            version,
            message: message.to_string(),
            timestamp: now_unix(),
        };
        let ch = hash_commit(&commit);
        self.commits.insert(ch, commit);
        self.head = Some(ch);
        ch
    }

    /// Get the [`Section`] for a commit.
    pub fn section(&self, commit: Hash) -> &Section {
        &self.sections[&self.commits[&commit].version]
    }

    /// Get the [`TreeNode`] for a commit.
    pub fn tree(&self, commit: Hash) -> TreeNode {
        self.section(commit).to_tree().unwrap_or_default()
    }

    /// Diff two commits: returns the [`Patch`] from `a` to `b`.
    pub fn diff(&self, a: Hash, b: Hash) -> Patch {
        patch::diff_sections(self.section(a), self.section(b))
    }

    /// Three-way merge of two commits via their lowest common ancestor.
    /// Merge is the pushout of `ours <- base -> theirs`.
    pub fn merge(&mut self, ours: Hash, theirs: Hash) -> MergeResult {
        let base = self.merge_base(ours, theirs);
        let result = merge_sections(self.section(base), self.section(ours), self.section(theirs));
        let merged_section = self.store_section(result.merged);
        MergeResult {
            merged_section,
            conflicts: result.conflicts,
            base,
            ours,
            theirs,
        }
    }

    /// Cherry-pick: apply the change `src_parent -> src` onto `dst`.
    pub fn cherry_pick(&mut self, src: Hash, dst: Hash) -> Result<Hash, ApplyError> {
        let parent = self.commits[&src].parents.first().copied();
        let p = match parent {
            Some(p) => self.diff(p, src),
            None => patch::diff_sections(&Section::new(), self.section(src)),
        };
        match patch::apply_to_section(&p, self.section(dst)) {
            Ok(merged) => Ok(self.store_section(merged)),
            Err(e) => Err(*e),
        }
    }

    /// Revert: produce the inverse of `target` relative to its parent, apply to `dst`.
    pub fn revert(&mut self, target: Hash, dst: Hash) -> Result<Hash, ApplyError> {
        let parent = self.commits[&target].parents.first().copied();
        let p = match parent {
            Some(p) => self.diff(p, target),
            None => patch::diff_sections(self.section(target), &Section::new()),
        };
        let inv = patch::invert(&p);
        match patch::apply_to_section(&inv, self.section(dst)) {
            Ok(merged) => Ok(self.store_section(merged)),
            Err(e) => Err(*e),
        }
    }

    /// Lowest common ancestor of two commits in the DAG.
    pub fn merge_base(&self, a: Hash, b: Hash) -> Hash {
        if a == b {
            return a;
        }
        let anc_a = self.all_ancestors(a);
        // BFS from b toward roots; return the first commit in anc_a that is
        // not a or b themselves.
        let mut queue = vec![b];
        let mut visited = HashSet::new();
        let mut best: Option<(Hash, usize)> = None; // (hash, depth)
        while let Some(c) = queue.pop() {
            if !visited.insert(c) {
                continue;
            }
            if c != a && c != b && anc_a.contains(&c) {
                let depth = self.depth(c);
                match best {
                    None => best = Some((c, depth)),
                    Some((_, bd)) if depth < bd => best = Some((c, depth)),
                    _ => {}
                }
            }
            if let Some(commit) = self.commits.get(&c) {
                for &p in &commit.parents {
                    queue.push(p);
                }
            }
        }
        best.map(|(h, _)| h)
            .unwrap_or_else(|| if anc_a.contains(&b) { b } else { a })
    }

    fn all_ancestors(&self, commit: Hash) -> HashSet<Hash> {
        let mut set = HashSet::new();
        let mut stack = vec![commit];
        while let Some(c) = stack.pop() {
            if set.insert(c) {
                if let Some(commit) = self.commits.get(&c) {
                    stack.extend(&commit.parents);
                }
            }
        }
        set
    }

    fn depth(&self, commit: Hash) -> usize {
        let mut max = 0;
        let mut visited = HashSet::new();
        let mut stack = vec![(commit, 0usize)];
        while let Some((c, d)) = stack.pop() {
            if !visited.insert(c) {
                continue;
            }
            max = max.max(d);
            if let Some(commit) = self.commits.get(&c) {
                for &p in &commit.parents {
                    stack.push((p, d + 1));
                }
            }
        }
        max
    }

    /// Commit history from a given commit (or HEAD) to root, in topological order.
    pub fn log(&self, from: Option<Hash>) -> Vec<(Hash, &Commit)> {
        let start = match from.or(self.head) {
            Some(h) => h,
            None => return Vec::new(),
        };
        let mut visited = HashSet::new();
        let mut result = Vec::new();
        let mut stack = vec![start];
        while let Some(c) = stack.pop() {
            if !visited.insert(c) {
                continue;
            }
            if let Some(commit) = self.commits.get(&c) {
                result.push((c, commit));
                stack.extend(&commit.parents);
            }
        }
        result
    }

    /// Create or move a named branch.
    pub fn branch(&mut self, name: &str, commit: Hash) {
        self.branches.insert(name.to_string(), commit);
    }

    /// Get the commit a branch points to.
    pub fn branch_head(&self, name: &str) -> Option<Hash> {
        self.branches.get(name).copied()
    }

    /// List all branches.
    pub fn branches(&self) -> &HashMap<String, Hash> {
        &self.branches
    }

    /// Current HEAD.
    pub fn head(&self) -> Option<Hash> {
        self.head
    }

    /// Set HEAD.
    pub fn set_head(&mut self, commit: Hash) {
        self.head = Some(commit);
    }

    /// Total number of commits.
    pub fn len(&self) -> usize {
        self.commits.len()
    }

    /// True if the repo has no commits.
    pub fn is_empty(&self) -> bool {
        self.commits.is_empty()
    }
}

impl Default for Repo {
    fn default() -> Self {
        Self::new()
    }
}

// ─── Deterministic hashing (FNV-1a, no external dependency) ───────────────

fn fnv1a(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn hash_section(s: &Section) -> Hash {
    let mut buf = Vec::new();
    for (loc, val) in &s.values {
        for seg in loc {
            buf.extend_from_slice(seg.as_bytes());
            buf.push(0);
        }
        buf.push(b'|');
        buf.extend(format!("{:?}", val).as_bytes());
        buf.push(b'\n');
    }
    fnv1a(&buf)
}

fn hash_commit(c: &Commit) -> Hash {
    let mut buf = Vec::new();
    for &p in &c.parents {
        buf.extend_from_slice(&p.to_le_bytes());
    }
    buf.extend_from_slice(&c.version.to_le_bytes());
    buf.extend_from_slice(c.message.as_bytes());
    buf.extend_from_slice(&c.timestamp.to_le_bytes());
    fnv1a(&buf)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TreeNode {
        TreeNode::new("root")
            .with_child(
                TreeNode::new("server").with_identity("s1")
                    .with_attr("port", "8080")
                    .with_attr("host", "localhost"),
            )
            .with_child(
                TreeNode::new("db").with_identity("d1")
                    .with_attr("url", "postgres://localhost"),
            )
    }

    #[test]
    fn commit_and_log() {
        let mut repo = Repo::new();
        let v0 = repo.commit("initial", &[], &sample());
        assert_eq!(repo.len(), 1);

        let log = repo.log(None);
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].1.message, "initial");
    }

    #[test]
    fn diff_two_commits() {
        let mut repo = Repo::new();
        let v0 = repo.commit("v0", &[], &sample());

        let mut modified = sample();
        modified.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port change", &[v0], &modified);

        let patch = repo.diff(v0, v1);
        assert!(!patch.is_empty());
    }

    #[test]
    fn clean_merge_disjoint_changes() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());

        // Ours: change port.
        let mut ours_tree = sample();
        ours_tree.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port", &[v0], &ours_tree);

        // Theirs: change db url.
        let mut theirs_tree = sample();
        theirs_tree.children[1].attributes[0].1 = "postgres://prod".into();
        let v2 = repo.commit("db url", &[v0], &theirs_tree);

        let result = repo.merge(v1, v2);
        assert!(result.is_clean(), "disjoint changes must merge cleanly");

        let merged = &repo.sections[&result.merged_section];
        // Both changes present.
        let server = merged.values.values().find(|v| v.kind == "server").unwrap();
        assert_eq!(server.attrs.iter().find(|(k, _)| k == "port").unwrap().1, "9090");
        let db = merged.values.values().find(|v| v.kind == "db").unwrap();
        assert_eq!(db.attrs.iter().find(|(k, _)| k == "url").unwrap().1, "postgres://prod");
    }

    #[test]
    fn conflict_merge_same_field() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());

        let mut ours_tree = sample();
        ours_tree.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port 9090", &[v0], &ours_tree);

        let mut theirs_tree = sample();
        theirs_tree.children[0].attributes[0].1 = "3000".into();
        let v2 = repo.commit("port 3000", &[v0], &theirs_tree);

        let result = repo.merge(v1, v2);
        assert!(!result.is_clean(), "same field changed differently must conflict");
        assert_eq!(result.conflicts.len(), 1);
    }

    #[test]
    fn cherry_pick_works() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());

        // Create change on one branch.
        let mut changed = sample();
        changed.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port change", &[v0], &changed);

        // Create unrelated change on another branch.
        let mut other = sample();
        other.children[1].attributes[0].1 = "postgres://staging".into();
        let v2 = repo.commit("staging db", &[v0], &other);

        // Cherry-pick port change onto staging branch.
        let picked = repo.cherry_pick(v1, v2).expect("cherry-pick should apply");
        let section = &repo.sections[&picked];
        let server = section.values.values().find(|v| v.kind == "server").unwrap();
        assert_eq!(server.attrs.iter().find(|(k, _)| k == "port").unwrap().1, "9090");
        let db = section.values.values().find(|v| v.kind == "db").unwrap();
        assert_eq!(db.attrs.iter().find(|(k, _)| k == "url").unwrap().1, "postgres://staging");
    }

    #[test]
    fn revert_works() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());

        let mut changed = sample();
        changed.children[0].attributes[0].1 = "9090".into();
        let v1 = repo.commit("port change", &[v0], &changed);

        let reverted = repo.revert(v1, v1).expect("revert should apply");
        let section = &repo.sections[&reverted];
        let server = section.values.values().find(|v| v.kind == "server").unwrap();
        assert_eq!(server.attrs.iter().find(|(k, _)| k == "port").unwrap().1, "8080");
    }

    #[test]
    fn branches() {
        let mut repo = Repo::new();
        let v0 = repo.commit("initial", &[], &sample());
        let v1 = repo.commit("second", &[v0], &sample());

        repo.branch("main", v0);
        repo.branch("dev", v1);
        assert_eq!(repo.branch_head("main"), Some(v0));
        assert_eq!(repo.branch_head("dev"), Some(v1));
        assert_eq!(repo.branches().len(), 2);
    }

    #[test]
    fn merge_base_is_common_ancestor() {
        let mut repo = Repo::new();
        let v0 = repo.commit("root", &[], &sample());
        let v1 = repo.commit("a", &[v0], &sample());
        let v2 = repo.commit("b", &[v0], &sample());
        let v3 = repo.commit("c", &[v1], &sample());

        let mb = repo.merge_base(v3, v2);
        assert_eq!(mb, v0, "LCA of (c, b) should be root");
    }

    #[test]
    fn identical_branches_merge_clean() {
        let mut repo = Repo::new();
        let v0 = repo.commit("base", &[], &sample());
        let v1 = repo.commit("same change", &[v0], &sample());
        let v2 = repo.commit("same change", &[v0], &sample());
        let result = repo.merge(v1, v2);
        assert!(result.is_clean());
    }
}
