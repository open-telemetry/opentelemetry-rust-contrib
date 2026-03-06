use std::{borrow::Borrow, cmp::Ordering, collections::HashMap};

use crate::xray_exporter::{
    error::ConstraintError,
    types::{DocumentBuilderHeader, Id, TraceId},
};

/// Unique identifier for a node within the tree.
///
/// This is an index into the tree's node vector, wrapped in a newtype for type safety.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NodeId(usize);

/// A node in the tree structure with bidirectional sibling and cousin links.
///
/// This structure maintains multiple types of relationships:
/// - **Parent-child**: Via `first_child` and `last_child` pointers
/// - **Sibling**: Nodes that share the same parent, linked via `previous_sibling` and `next_sibling`
/// - **Cousin**: Nodes at the same level but with different parents, linked via `previous_cousin` and `next_cousin`
///
/// The cousin links enable efficient level-order traversal without needing to track parent relationships.
/// Siblings are kept sorted according to the data's [`Ord`] implementation.
struct TreeNode<T> {
    /// Depth of this node in the tree (root nodes have level 0).
    level: usize,
    /// Parent of this node, if any.
    parent: Option<NodeId>,
    /// First child of this node, if any.
    first_child: Option<NodeId>,
    /// Last child of this node, if any.
    last_child: Option<NodeId>,
    /// Previous sibling (same parent, earlier in sort order).
    previous_sibling: Option<NodeId>,
    /// Next sibling (same parent, later in sort order).
    next_sibling: Option<NodeId>,
    /// Previous node at the same level that is not a sibling.
    previous_cousin: Option<NodeId>,
    /// Next node at the same level that is not a sibling.
    next_cousin: Option<NodeId>,
    /// The data stored in this node.
    data: T,
}

impl<T> core::fmt::Debug for TreeNode<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TreeNode")
            .field("level", &self.level)
            .field("parent", &self.parent)
            .field("first_child", &self.first_child)
            .field("last_child", &self.last_child)
            .field("previous_sibling", &self.previous_sibling)
            .field("next_sibling", &self.next_sibling)
            .field("previous_cousin", &self.previous_cousin)
            .field("next_cousin", &self.next_cousin)
            .finish()
    }
}

impl<T> TreeNode<T> {
    /// Creates a new tree node at the specified level with the given data.
    ///
    /// All link fields are initialized to `None`.
    fn new(level: usize, data: T) -> Self {
        Self {
            level,
            parent: None,
            first_child: None,
            last_child: None,
            previous_sibling: None,
            next_sibling: None,
            previous_cousin: None,
            next_cousin: None,
            data,
        }
    }
}
/// Tracks the first and last nodes at a specific level in the tree.
///
/// This enables efficient level-order traversal by providing direct access to the start
/// of each level, and efficient level-order addition by providing direct access to the end
/// of each level, all without needing to traverse parent-child relationships.
#[derive(Debug, Clone, Copy)]
struct TreeLevel {
    /// The first node at this level.
    first: NodeId,
    /// The last node at this level.
    last: NodeId,
}
/// A tree structure with sorted siblings and efficient level-order traversal.
///
/// This tree maintains several invariants:
/// - Nodes are stored in a flat vector for cache-friendly access
/// - Siblings are kept sorted according to their data's [`Ord`] implementation
/// - Each level maintains pointers to its first and last nodes
/// - Cousin links connect nodes at the same level across different parent subtrees
///
/// The tree supports dynamic reparenting of nodes while maintaining all structural invariants.
#[derive(Debug)]
struct Tree<T> {
    /// Flat storage of all tree nodes.
    nodes: Vec<TreeNode<T>>,
    /// Level metadata for efficient traversal.
    levels: Vec<TreeLevel>,
}

impl<T> Tree<T> {
    /// Returns an immutable reference to the node with the given ID.
    fn get(&self, node_id: NodeId) -> &TreeNode<T> {
        &self.nodes[node_id.0]
    }

    /// Returns a mutable reference to the node with the given ID.
    fn _get_mut(&mut self, node_id: NodeId) -> &mut TreeNode<T> {
        &mut self.nodes[node_id.0]
    }

    /// Returns mutable references to two different nodes.
    ///
    /// Uses `split_at_mut` to safely obtain two mutable references from the same vector.
    ///
    /// # Panics
    ///
    /// Panics if both node IDs are the same, as this would create two mutable references
    /// to the same node.
    fn _get_mut_nodes(
        &mut self,
        node1_id: NodeId,
        node2_id: NodeId,
    ) -> (&mut TreeNode<T>, &mut TreeNode<T>) {
        match node1_id.0.cmp(&node2_id.0) {
            Ordering::Less => {
                let (s1, s2) = self.nodes.split_at_mut(node2_id.0);
                (&mut s1[node1_id.0], &mut s2[0])
            }
            Ordering::Greater => {
                let (s1, s2) = self.nodes.split_at_mut(node1_id.0);
                (&mut s2[0], &mut s1[node2_id.0])
            }
            Ordering::Equal => panic!("Cannot create 2 mut ref for the same node"),
        }
    }

    /// Inserts a node immediately before an existing node in the sibling list.
    ///
    /// This operation maintains all structural invariants:
    /// - Updates sibling links for the new node, existing node, and adjacent siblings
    /// - Transfers the existing node's previous cousin link to the new node
    /// - Clears the existing node's previous cousin link (no longer first in its sibling group)
    /// - Updates level metadata if the new node becomes the first at its level
    ///
    /// The new node takes over the existing node's position in the cousin chain (if it has any)
    fn _insert_node_before(&mut self, node_id: NodeId, existing_node_id: NodeId) {
        // Get mutable references to both nodes and extract values we need
        let (node, existing_node) = self._get_mut_nodes(node_id, existing_node_id);

        // Link with the parent
        node.parent = existing_node.parent;

        // Link the new node into the sibling chain
        node.previous_sibling = existing_node.previous_sibling;
        node.next_sibling = Some(existing_node_id);
        existing_node.previous_sibling = Some(node_id);

        // Transfer cousin link from existing node to new node
        node.previous_cousin = existing_node.previous_cousin;
        existing_node.previous_cousin = None; // Can no longer be the first in its sibling group

        // Extract values before updating other nodes
        let level = node.level;
        let previous_sibling = node.previous_sibling;
        let previous_cousin = node.previous_cousin;
        let parent = node.parent.expect("siblings have a parent by definition");

        // Update the previous sibling's forward link
        if let Some(previous_sibling) = previous_sibling {
            self._get_mut(previous_sibling).next_sibling = Some(node_id);
        }

        // Update the previous cousin's forward link
        if let Some(previous_cousin) = previous_cousin {
            self._get_mut(previous_cousin).next_cousin = Some(node_id);
        }

        // Update level metadata if this becomes the first node at this level
        if self.levels[level].first == existing_node_id {
            self.levels[level].first = node_id;
        }

        // Update parent if this becomes the first sibling
        let parent_node = self._get_mut(parent);
        if parent_node.first_child.expect("this parent have children") == existing_node_id {
            parent_node.first_child = Some(node_id);
        }
    }

    /// Inserts a node immediately after an existing node in the sibling list.
    ///
    /// This is the mirror operation of `_insert_node_before`, maintaining all structural invariants:
    /// - Updates sibling links for the new node, existing node, and adjacent siblings
    /// - Transfers the existing node's next cousin link to the new node
    /// - Clears the existing node's next cousin link (no longer last in its sibling group)
    /// - Updates level metadata if the new node becomes the last at its level
    ///
    /// The new node takes over the existing node's position in the cousin chain (if it has any).
    fn _insert_node_after(&mut self, node_id: NodeId, existing_node_id: NodeId) {
        // Get mutable references to both nodes and extract values we need
        let (node, existing_node) = self._get_mut_nodes(node_id, existing_node_id);

        // Link with the parent
        node.parent = existing_node.parent;

        // Link the new node into the sibling chain
        node.next_sibling = existing_node.next_sibling;
        node.previous_sibling = Some(existing_node_id);
        existing_node.next_sibling = Some(node_id);

        // Transfer cousin link from existing node to new node
        node.next_cousin = existing_node.next_cousin;
        existing_node.next_cousin = None; // No longer the last in its sibling group

        // Extract values before updating other nodes
        let level = node.level;
        let next_sibling = node.next_sibling;
        let next_cousin = node.next_cousin;
        let parent = node.parent.expect("siblings have a parent by definition");

        // Update the next sibling's backward link
        if let Some(next_sibling) = next_sibling {
            self._get_mut(next_sibling).previous_sibling = Some(node_id);
        }

        // Update the next cousin's backward link
        if let Some(next_cousin) = next_cousin {
            self._get_mut(next_cousin).previous_cousin = Some(node_id);
        }

        // Update level metadata if this becomes the last node at this level
        if self.levels[level].last == existing_node_id {
            self.levels[level].last = node_id;
        }

        // Update parent if this becomes the last sibling
        let parent_node = self._get_mut(parent);
        if parent_node.last_child.expect("this parent have children") == existing_node_id {
            parent_node.last_child = Some(node_id);
        }
    }

    /// Appends a sibling group to the end of a level's cousin chain.
    ///
    /// If the level already exists, the sibling group is linked after the current last node.
    /// If the level doesn't exist yet, a new level is created with this sibling group.
    ///
    /// This is used when adding nodes that should appear at the end of their level,
    /// such as when inserting a new root node, a first child or moving existing children.
    fn _append_siblings_at_lvl(&mut self, first_sibling_id: NodeId, last_sibling_id: NodeId) {
        let level = self.get(first_sibling_id).level;
        if let Some(lvl) = self.levels.get_mut(level) {
            // Level exists, append siblings to the end
            let previous_last = lvl.last;

            lvl.last = last_sibling_id;

            self._get_mut(previous_last).next_cousin = Some(first_sibling_id);
            self._get_mut(first_sibling_id).previous_cousin = Some(previous_last);
        } else {
            // New level
            self.levels.push(TreeLevel {
                first: first_sibling_id,
                last: last_sibling_id,
            });
        }
    }

    /// Removes a sibling group from its level's cousin chain.
    ///
    /// This operation maintains all structural invariants:
    /// - Disconnects the sibling group from the cousin chain by updating adjacent cousins
    /// - Updates level metadata if the group contains the first or last node at its level
    /// - Disconnects the sibling group from its parent's child list
    /// - Clears the sibling group's cousin and sibling links (except internal sibling links)
    ///
    /// This is used when reparenting nodes to a different level or parent.
    ///
    /// # Panics
    ///
    /// Panics if the sibling group represents an entire level (both first and last of level
    /// with no cousins), as this would leave the level empty.
    fn _remove_siblings_from_lvl(&mut self, first_sibling_id: NodeId, last_sibling_id: NodeId) {
        let first_sibling = self._get_mut(first_sibling_id);
        let previous_sibling = first_sibling.previous_sibling;
        let previous_cousin = first_sibling.previous_cousin;
        first_sibling.previous_sibling = None;
        first_sibling.previous_cousin = None;
        let last_sibling = self._get_mut(last_sibling_id);
        let next_sibling = last_sibling.next_sibling;
        let next_cousin = last_sibling.next_cousin;
        last_sibling.next_sibling = None;
        last_sibling.next_cousin = None;
        let parent_id = last_sibling.parent;
        let level = last_sibling.level;
        let level = &mut self.levels[level];

        match (previous_cousin, next_cousin) {
            (Some(previous_cousin), Some(next_cousin)) => {
                // Siblings are is in the middle of the cousin chain
                self._get_mut(previous_cousin).next_cousin = Some(next_cousin);
                self._get_mut(next_cousin).previous_cousin = Some(previous_cousin);
            }
            (Some(previous_cousin), None) => {
                // Siblings only have a previous_cousin
                if let Some(next_sibling) = next_sibling {
                    // Either they have a next sibling
                    self._get_mut(previous_cousin).next_cousin = Some(next_sibling);
                    self._get_mut(next_sibling).previous_cousin = Some(previous_cousin);
                } else if level.last == last_sibling_id {
                    // Or they were the last of their level
                    level.last = previous_cousin;
                    self._get_mut(previous_cousin).next_cousin = None;
                } else {
                    unreachable!("would violate tree invariants");
                }
            }
            (None, Some(next_cousin)) => {
                // Siblings only have a next_cousin
                if let Some(previous_sibling) = previous_sibling {
                    // Either they have a previous sibling
                    self._get_mut(next_cousin).previous_cousin = Some(previous_sibling);
                    self._get_mut(previous_sibling).next_cousin = Some(next_cousin);
                } else if level.first == first_sibling_id {
                    // Or they were the first of their level
                    level.first = next_cousin;
                    self._get_mut(next_cousin).previous_cousin = None;
                } else {
                    unreachable!("would violate tree invariants");
                }
            }
            (None, None) => {
                // Siblings don't have any cousin
                if level.first == first_sibling_id && level.last == last_sibling_id {
                    // This would mean out sibling chain represent the entire level
                    panic!("cannot be used to remove an entire level");
                } else if level.first == first_sibling_id {
                    level.first = next_sibling.expect("would violate tree invariants");
                } else if level.last == last_sibling_id {
                    level.last = previous_sibling.expect("would violate tree invariants");
                }
            }
        }

        // Their could be no parent if the medthod was called on a single node (first_sibling_id == last_sibling_id)
        // and that node was a root node
        if let Some(parent_id) = parent_id {
            let parent = self._get_mut(parent_id);
            let first_child = parent.first_child.expect("this parent have children");
            let last_child = parent.last_child.expect("this parent have children");
            match (previous_sibling, next_sibling) {
                (Some(previous_sibling), Some(next_sibling)) => {
                    // Siblings are in the middle of the sibling chain
                    self._get_mut(previous_sibling).next_sibling = Some(next_sibling);
                    self._get_mut(next_sibling).previous_sibling = Some(previous_sibling);
                }
                (Some(previous_sibling), None) if last_child == last_sibling_id => {
                    // Siblings are at the end of the sibling chain
                    parent.last_child = Some(previous_sibling);
                    self._get_mut(previous_sibling).next_sibling = None;
                }
                (None, Some(next_sibling)) if first_child == first_sibling_id => {
                    // Siblings are at the begining of the sibling chain
                    parent.first_child = Some(next_sibling);
                    self._get_mut(next_sibling).previous_sibling = None;
                }
                (None, None)
                    if first_child == first_sibling_id && last_child == last_sibling_id => {}
                _ => {
                    unreachable!("would violate tree invariants");
                }
            }
        }
    }

    /// Updates the level of all nodes in a sibling chain.
    ///
    /// Traverses the sibling chain from `first_sibling_id` to `last_sibling_id` and sets
    /// each node's level to `new_level`. This is used during reparenting operations to
    /// adjust node depths after moving a subtree to a new parent.
    ///
    /// The sibling links themselves are not modified, only the level field.
    fn _update_siblings_lvl(
        &mut self,
        first_sibling_id: NodeId,
        last_sibling_id: NodeId,
        new_level: usize,
    ) {
        let mut cursor = first_sibling_id;
        loop {
            let node = self._get_mut(cursor);
            node.level = new_level;
            if cursor == last_sibling_id {
                break;
            }
            if let Some(next) = node.next_sibling {
                cursor = next;
            } else {
                break;
            }
        }
    }

    /// Recursively updates the level of a node's entire subtree.
    ///
    /// This is called after a node has been moved to a new level (via reparenting) to ensure
    /// all descendants are also updated to maintain the correct depth relationship.
    ///
    /// The process:
    /// 1. Removes the node's children from their current level
    /// 2. Updates all children to the correct level (parent's level + 1)
    /// 3. Re-adds the children to their new level
    /// 4. Recursively processes each child's subtree
    ///
    /// If the node has no children, this is a no-op.
    fn _move_node_subtree(&mut self, node_id: NodeId) {
        // extract values we need
        let node = self.get(node_id);
        let level = node.level;
        let first_child_id = node.first_child;
        let last_child_id = node.last_child;

        if let (Some(first_child_id), Some(last_child_id)) = (first_child_id, last_child_id) {
            // Modify children level
            self._remove_siblings_from_lvl(first_child_id, last_child_id);
            self._update_siblings_lvl(first_child_id, last_child_id, level + 1);
            self._append_siblings_at_lvl(first_child_id, last_child_id);

            // Recursively call the function for each child
            let mut child = first_child_id;
            loop {
                self._move_node_subtree(child);
                if let Some(next) = self.get(child).next_sibling {
                    child = next;
                } else {
                    break;
                }
            }
        }
    }

    /// Updates the data stored in a node without changing its position in the tree.
    ///
    /// This is used to fill in placeholder nodes when the actual span data arrives.
    fn set_node_data(&mut self, node_id: NodeId, data: T) {
        // Update the data
        self._get_mut(node_id).data = data;
    }
}

impl<T: Ord> Tree<T> {
    /// Inserts a node as a sibling under the specified parent, maintaining sort order.
    ///
    /// Uses a bidirectional search algorithm to find the insertion point efficiently:
    /// - Searches forward from the first child and backward from the last child simultaneously
    /// - Inserts the node as soon as the correct position is found
    /// - Updates parent's first/last child pointers if necessary
    ///
    /// If the parent has no children yet, the new node becomes both the first and last child,
    /// and is registered at its level via `_append_siblings_at_lvl`.
    ///
    /// The bidirectional search is more efficient than linear search for large sibling lists,
    /// especially when the insertion point is near either end.
    fn _insert_sibling_sorted(&mut self, node_id: NodeId, parent_node_id: NodeId) {
        let parent_node = self._get_mut(parent_node_id);

        match (parent_node.first_child, parent_node.last_child) {
            (Some(first_child), Some(last_child)) => {
                let node_data = &self.get(node_id).data;
                // Parent has existing children, find the sorted insertion point
                let mut forward_cursor = first_child;
                let mut backward_cursor = last_child;

                loop {
                    // Check forward cursor
                    let forward_node = self.get(forward_cursor);
                    if *node_data < forward_node.data {
                        // Found insertion point: before forward_cursor
                        self._insert_node_before(node_id, forward_cursor);
                        break;
                    }

                    // If forward cursor is at the end, insert after it
                    if forward_node.next_sibling.is_none() {
                        self._insert_node_after(node_id, forward_cursor);
                        break;
                    }
                    forward_cursor = forward_node.next_sibling.unwrap();

                    // Check backward cursor
                    let backward_node = self.get(backward_cursor);
                    if *node_data >= backward_node.data {
                        // Found insertion point: after backward_cursor
                        self._insert_node_after(node_id, backward_cursor);
                        break;
                    }
                    backward_cursor = backward_node.previous_sibling.expect("Cannot be None because forward_node.next_sibling would also be None so we already break");
                }
            }
            (None, None) => {
                // Parent has no children, this becomes the only child
                parent_node.first_child = Some(node_id);
                parent_node.last_child = Some(node_id);
                self._get_mut(node_id).parent = Some(parent_node_id);
                self._append_siblings_at_lvl(node_id, node_id);
            }
            _ => unreachable!("first and last child are always both None or both Some"),
        }
    }

    /// Inserts a new node into the tree with the specified parent.
    ///
    /// The insertion process:
    /// 1. Creates a new node with the appropriate level (parent's level + 1, or 0 for root)
    /// 2. Pushes the node to storage to allocate its ID
    /// 3. If a parent is specified, inserts the node into the parent's sorted child list
    /// 4. If no parent, appends the node to level 0 as a root node
    ///
    /// Returns the [`NodeId`] of the newly inserted node.
    fn insert_sorted(&mut self, data: T, parent_node_id: Option<NodeId>) -> NodeId {
        // Allocate the node ID before insertion
        let node_id = NodeId(self.nodes.len());

        let node = if let Some(parent_node_id) = parent_node_id {
            // Create as a child of the specified parent
            TreeNode::new(self.get(parent_node_id).level + 1, data)
        } else {
            // Create as a root node at level 0
            TreeNode::new(0, data)
        };

        // Add the node to storage
        self.nodes.push(node);

        if let Some(parent_node_id) = parent_node_id {
            // Insert into parent's sorted child list
            self._insert_sibling_sorted(node_id, parent_node_id);
        } else {
            // Append to level 0 as a root node
            self._append_siblings_at_lvl(node_id, node_id);
        };
        node_id
    }

    /// Moves a node and its entire subtree under a new parent, maintaining sort order.
    ///
    /// This operation:
    /// 1. Removes the node from its current level and parent
    /// 2. Updates the node's level to match the new parent's depth + 1
    /// 3. Inserts the node into the new parent's sorted child list
    /// 4. Recursively updates the level of all descendants
    ///
    /// This is used when a placeholder parent is filled in with actual data that specifies
    /// a different parent, requiring the entire subtree to be moved.
    fn reparent_sorted(&mut self, node_id: NodeId, new_parent_node_id: NodeId) {
        // Reparent the node under a new parent
        let parent_level = self.get(new_parent_node_id).level;
        // Remove the node from its level
        self._remove_siblings_from_lvl(node_id, node_id);

        self._update_siblings_lvl(node_id, node_id, parent_level + 1);

        // Insert into the new parent's sibling list
        self._insert_sibling_sorted(node_id, new_parent_node_id);

        // Recursively update level of this node and all descendants
        self._move_node_subtree(node_id);
    }
}

/// A simple wrapper around [`HashMap`] for indexing tree nodes by their identifiers.
///
/// This abstraction allows for easy replacement of the underlying data structure
/// for benchmarking purposes without changing the rest of the code.
struct Index<K, V> {
    inner: HashMap<K, V>,
}

impl<K: Eq + std::hash::Hash, V> Index<K, V> {
    /// Inserts a key-value pair into the index, replacing any existing value.
    fn insert(&mut self, key: K, value: V) {
        self.inner.insert(key, value);
    }

    /// Retrieves a reference to the value associated with the given key.
    ///
    /// Returns `None` if the key is not present in the index.
    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: ?Sized + Eq + std::hash::Hash,
    {
        self.inner.get(key)
    }
}

/// Wrapper around [`DocumentBuilderHeader`] that implements [`Ord`] for tree sorting.
///
/// This wrapper enables sorting span headers by start time in the tree structure.
/// Headers are compared by their start time, with `None` values (placeholders) sorting
/// before any actual start time.
///
/// The inner `Option` allows for placeholder nodes that will be filled in later when
/// the actual span data arrives.
#[derive(Debug)]
struct DocumentBuilderHeaderWrapper(Option<DocumentBuilderHeader>);

impl Eq for DocumentBuilderHeaderWrapper {}

impl PartialEq for DocumentBuilderHeaderWrapper {
    /// Compares two headers for equality based on their start times.
    ///
    /// Two headers are equal if they have the same start time, or if both are placeholders (`None`).
    fn eq(&self, other: &Self) -> bool {
        self.0.map(|h| h.start_time) == other.0.map(|h| h.start_time)
    }
}
impl Ord for DocumentBuilderHeaderWrapper {
    /// Orders headers by start time for chronological sorting in the tree.
    ///
    /// Uses `total_cmp` for floating-point comparison to handle NaN consistently
    fn cmp(&self, other: &Self) -> Ordering {
        match (
            self.0.and_then(|h| h.start_time),
            other.0.and_then(|h| h.start_time),
        ) {
            (Some(h1), Some(h2)) => h1.total_cmp(&h2),
            _ => unreachable!("Placeholders never have parent, so no siblings to sort either"),
        }
    }
}
impl PartialOrd for DocumentBuilderHeaderWrapper {
    /// Delegates to the total ordering defined by [`Ord::cmp`].
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A tree structure for organizing X-Ray span documents by parent-child relationships.
///
/// This structure solves the problem of receiving spans in arbitrary order while needing to
/// export them in a hierarchical, time-ordered format when the translator's `always_nest_subsegments`
/// flag is set.
///
/// Key features:
///
/// - **Placeholder nodes**: When a span references a parent that hasn't been seen yet, a placeholder
///   node with `None` data is created. When the parent span arrives, the placeholder is filled in.
///
/// - **Dynamic reparenting**: If a placeholder is filled in with data that specifies a different parent
///   than initially assumed (root), the entire subtree is reparented to maintain the correct hierarchy.
///
/// - **Sorted siblings**: Spans with the same parent are kept sorted by start time, ensuring
///   chronological ordering within each level of the trace tree.
///
/// - **Efficient lookup**: The `node_index` provides O(1) lookup of nodes by (TraceId, SpanId) pair.
///
/// The tree maintains cousin links between nodes at the same level, enabling efficient bottom-up
/// traversal for export (children are exported before their parents).
pub(super) struct DocumentBuilderHeaderTree {
    /// The underlying tree structure storing span headers.
    header_tree: Tree<DocumentBuilderHeaderWrapper>,
    /// Index mapping (TraceId, SpanId) pairs to their node IDs.
    node_index: Index<(TraceId, Id), NodeId>,
}

impl DocumentBuilderHeaderTree {
    /// Creates a new empty tree with pre-allocated capacity.
    ///
    /// The `size` parameter is used to pre-allocate storage for the expected number of spans,
    /// reducing allocations during tree construction.
    ///
    /// Note that the creation of placeholder parents can lead to the tree growing to twice the
    /// initially allocated size in the worst case (if all the spans have different parent IDs that are
    /// not in the currently processed batch).
    pub fn new(size: usize) -> Self {
        Self {
            header_tree: Tree {
                nodes: Vec::with_capacity(size),
                levels: Vec::new(),
            },
            node_index: Index {
                inner: HashMap::with_capacity(size),
            },
        }
    }

    /// Adds a span header to the tree, maintaining parent-child relationships.
    ///
    /// This method handles several scenarios:
    ///
    /// 1. **New span with known parent**: If the parent exists in the tree, the span is inserted
    ///    as a child of that parent in sorted order.
    ///
    /// 2. **New span with unknown parent**: If the parent hasn't been seen yet, a placeholder node
    ///    is created for the parent, and the span is inserted as its child. When the parent span
    ///    arrives later, the placeholder will be filled in.
    ///
    /// 3. **Placeholder being filled**: If a node already exists for this span ID (created as a
    ///    placeholder when a child referenced it), the node is updated with the actual span data
    ///    and potentially reparented if the parent information differs.
    ///
    /// 4. **Root span**: If the span has no parent, it's inserted at the root level (level 0).
    ///
    /// # Errors
    ///
    /// Returns an error if the header is missing required fields (ID or TraceID).
    pub fn add(&mut self, header: DocumentBuilderHeader) -> super::Result<()> {
        // Extract required fields
        let id = header.id.ok_or(ConstraintError::MissingId)?;
        let trace_id = header.trace_id.ok_or(ConstraintError::MissingTraceId)?;
        let parent_id = header.parent_id;

        // Resolve or create the parent node if a parent ID is specified
        let parent_node_id = parent_id.map(|parent_id| {
            match self.node_index.get(&(trace_id, parent_id)).cloned() {
                Some(parent_node_id) => {
                    // Parent already exists in the tree
                    parent_node_id
                }
                None => {
                    // Parent not seen yet, create a placeholder node
                    let parent_node_id = self
                        .header_tree
                        .insert_sorted(DocumentBuilderHeaderWrapper(None), None);
                    self.node_index
                        .insert((trace_id, parent_id), parent_node_id);
                    parent_node_id
                }
            }
        });

        // Insert or update the node for this span
        match self.node_index.get(&(trace_id, id)).cloned() {
            Some(node_id) => {
                // Node already exists (was a placeholder), update it with actual data
                self.header_tree
                    .set_node_data(node_id, DocumentBuilderHeaderWrapper(Some(header)));
                if let Some(parent_node_id) = parent_node_id {
                    self.header_tree.reparent_sorted(node_id, parent_node_id);
                }
            }
            None => {
                // New node, insert it into the tree
                self.node_index.insert(
                    (trace_id, id),
                    self.header_tree
                        .insert_sorted(DocumentBuilderHeaderWrapper(Some(header)), parent_node_id),
                );
            }
        };
        Ok(())
    }

    /// Returns an iterator that traverses the tree in bottom-up, left-to-right order.
    ///
    /// The iterator starts at the deepest level and works upward, visiting siblings in
    /// chronological order (by start time). This ensures that child spans are yielded
    /// before their parents, which is required for X-Ray's export format.
    ///
    /// Placeholder nodes (with `None` data) are automatically skipped.
    pub fn iter(&self) -> DocumentBuilderHeaderTreeIterator {
        DocumentBuilderHeaderTreeIterator {
            tree: &self.header_tree,
            current_level: self.header_tree.levels.len() - 1,
            last_node_yield: None,
        }
    }
}

/// Iterator for traversing the document tree in bottom-up, left-to-right order.
///
/// This iterator yields [`DocumentBuilderHeader`] values in an order suitable for X-Ray export:
/// - Starts at the deepest level (leaf nodes)
/// - Within each level, visits nodes left-to-right (chronologically by start time via sibling links)
/// - Moves up to shallower levels after exhausting each level
/// - Skips placeholder nodes (those with `None` data)
///
/// The traversal uses the tree's sibling and cousin links for efficient navigation without
/// needing to maintain a separate traversal stack.
#[derive(Debug)]
pub(super) struct DocumentBuilderHeaderTreeIterator<'a> {
    /// Reference to the tree being iterated.
    tree: &'a Tree<DocumentBuilderHeaderWrapper>,
    /// Current level being traversed (decreases as we move up the tree).
    current_level: usize,
    /// The last node that was yielded, used to determine the next node to visit.
    last_node_yield: Option<NodeId>,
}

impl Iterator for DocumentBuilderHeaderTreeIterator<'_> {
    type Item = DocumentBuilderHeader;

    /// Returns the next document header in bottom-up, left-to-right order.
    ///
    /// The traversal logic:
    /// 1. If this is the first call, start at the first node of the deepest level
    /// 2. Otherwise, from the last yielded node:
    ///    - Try to move to the next sibling (same parent, next in time order)
    ///    - If no more siblings, try to move to the next cousin (same level, different parent)
    ///    - If no more cousins, move up one level and start at its first node
    ///    - If already at level 0 with no more nodes, iteration is complete
    /// 3. Skip nodes with `None` data (placeholders) and continue to the next node
    /// 4. Return the first node with actual data
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Determine the next node to visit
            let next_id = match self.last_node_yield {
                Some(last_node_yield) => {
                    let last_node = self.tree.get(last_node_yield);

                    if let Some(sibling) = last_node.next_sibling {
                        // Move to next sibling (same parent)
                        sibling
                    } else if let Some(cousin) = last_node.next_cousin {
                        // No more siblings, move to next cousin (same level, different parent)
                        cousin
                    } else if self.current_level > 0 {
                        // No more nodes at this level, move up one level
                        self.current_level -= 1;
                        self.tree.levels[self.current_level].first
                    } else {
                        // At level 0 with no more nodes, iteration complete
                        break None;
                    }
                }
                None => {
                    // First iteration, start at the first node of the deepest level
                    self.tree.levels[self.current_level].first
                }
            };

            // Update state
            self.last_node_yield = Some(next_id);

            // Skip placeholder nodes and return the first node with data
            if let Some(header) = self.tree.get(next_id).data.0 {
                break Some(header);
            }
            // If this node is a placeholder, continue to the next node
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test helpers

    /// Creates a test DocumentBuilderHeader with the given parameters
    fn create_test_header(
        id: u64,
        parent_id: Option<u64>,
        trace_id: u128,
        start_time: f64,
        end_time: f64,
    ) -> DocumentBuilderHeader {
        DocumentBuilderHeader {
            id: Some(Id::from(id)),
            parent_id: parent_id.map(Id::from),
            trace_id: Some(TraceId::from(trace_id)),
            start_time: Some(start_time),
            end_time: Some(end_time),
        }
    }

    /// Verifies all tree invariants hold
    fn verify_tree_invariants<T: core::fmt::Debug + Ord>(tree: &Tree<T>) {
        dbg!(&tree);
        // Check sibling chain bidirectional consistency
        for node_id in 0..tree.nodes.len() {
            let node = tree.get(NodeId(node_id));

            // Verify next_sibling's previous_sibling points back
            if let Some(next_sibling) = node.next_sibling {
                assert_eq!(
                    tree.get(next_sibling).previous_sibling,
                    Some(NodeId(node_id)),
                    "Sibling chain broken: next_sibling's previous_sibling doesn't point back"
                );
            }

            // Verify previous_sibling's next_sibling points back
            if let Some(previous_sibling) = node.previous_sibling {
                assert_eq!(
                    tree.get(previous_sibling).next_sibling,
                    Some(NodeId(node_id)),
                    "Sibling chain broken: previous_sibling's next_sibling doesn't point back"
                );
            }

            // Verify next_cousin's previous_cousin points back
            if let Some(next_cousin) = node.next_cousin {
                assert_eq!(
                    tree.get(next_cousin).previous_cousin,
                    Some(NodeId(node_id)),
                    "Cousin chain broken: next_cousin's previous_cousin doesn't point back"
                );
            }

            // Verify previous_cousin's next_cousin points back
            if let Some(previous_cousin) = node.previous_cousin {
                assert_eq!(
                    tree.get(previous_cousin).next_cousin,
                    Some(NodeId(node_id)),
                    "Cousin chain broken: previous_cousin's next_cousin doesn't point back"
                );
            }
        }

        // Check level metadata correctness
        for (level, TreeLevel { first, last }) in tree.levels.iter().enumerate() {
            let first_node = tree.get(*first);
            let last_node = tree.get(*last);

            assert_eq!(first_node.level, level, "Level first node has wrong level");
            assert_eq!(last_node.level, level, "Level last node has wrong level");

            // Verify first node has no previous_cousin
            assert!(
                first_node.previous_cousin.is_none(),
                "Level first node should have no previous_cousin"
            );

            // Verify last node has no next_cousin
            assert!(
                last_node.next_cousin.is_none(),
                "Level last node should have no next_cousin"
            );
        }

        // Check level consistency
        for node_id in 0..tree.nodes.len() {
            let node = tree.get(NodeId(node_id));
            // Verify children have correct level
            if let Some(first_child) = node.first_child {
                let mut sibling_cursor = first_child;
                loop {
                    let child = tree.get(sibling_cursor);
                    assert_eq!(
                        child.level,
                        node.level + 1,
                        "Child level should be parent level + 1"
                    );
                    if let Some(next) = child.next_sibling {
                        sibling_cursor = next;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    /// Verifies children of a parent are sorted
    fn assert_sibling_order<T: core::fmt::Debug + Ord>(tree: &Tree<T>, parent_id: NodeId) {
        let parent = tree.get(parent_id);

        if let Some(first_child) = parent.first_child {
            let mut current = first_child;
            let mut prev_data: Option<&T> = None;

            loop {
                let node = tree.get(current);

                if let Some(prev) = prev_data {
                    assert!(
                        node.data >= *prev,
                        "Siblings not sorted: {:?} < {:?}",
                        node.data,
                        prev
                    );
                }

                prev_data = Some(&node.data);

                if let Some(next) = node.next_sibling {
                    current = next;
                } else {
                    break;
                }
            }
        }
    }

    // Note: Tests for private Tree methods (_get_mut_nodes, _insert_node_before, _insert_node_after,
    // _append_siblings_at_lvl, _insert_sibling, _modify_node_level) are omitted because they are
    // implementation details. These methods are indirectly tested through the "public"
    // API tests below, which exercise all the tree operations.

    // Tests for Tree::insert (public within module)

    #[test]
    fn test_tree_insert_root_node() {
        let mut tree: Tree<u16> = Tree {
            nodes: Vec::new(),
            levels: Vec::new(),
        };

        // Insert root node (no parent)
        let node_id = tree.insert_sorted(0, None);

        // Verify node added
        assert_eq!(node_id, NodeId(0));
        assert_eq!(tree.nodes.len(), 1);

        // Verify level is 0
        assert_eq!(tree.get(node_id).level, 0);

        // Verify level 0 created
        assert_eq!(tree.levels.len(), 1);
        assert_eq!(tree.levels[0].first, node_id);
        assert_eq!(tree.levels[0].last, node_id);

        verify_tree_invariants(&tree);
    }

    #[test]
    fn test_tree_insert_child_node() {
        let mut tree: Tree<u16> = Tree {
            nodes: Vec::new(),
            levels: Vec::new(),
        };

        // Insert root node
        let root_id = tree.insert_sorted(0, None);

        // Insert child node
        let child_id = tree.insert_sorted(1, Some(root_id));

        // Verify child added
        assert_eq!(child_id, NodeId(1));
        assert_eq!(tree.nodes.len(), 2);

        // Verify level is parent.level + 1
        assert_eq!(tree.get(child_id).level, 1);

        // Verify parent's first_child and last_child
        assert_eq!(tree.get(root_id).first_child, Some(child_id));
        assert_eq!(tree.get(root_id).last_child, Some(child_id));

        verify_tree_invariants(&tree);
    }

    #[test]
    fn test_tree_insert_multiple_children() {
        let mut tree: Tree<u16> = Tree {
            nodes: Vec::new(),
            levels: Vec::new(),
        };

        // Insert root node
        let root_id = tree.insert_sorted(0, None);

        // Insert children with different start times
        let _child1_id = tree.insert_sorted(3, Some(root_id));
        let child2_id = tree.insert_sorted(2, Some(root_id));
        let child3_id = tree.insert_sorted(4, Some(root_id));

        // Verify all children added
        assert_eq!(tree.nodes.len(), 4);

        // Verify children sorted : child2 (2) -> child1 (3) -> child3 (4)
        assert_sibling_order(&tree, root_id);

        // Verify parent's first_child and last_child
        assert_eq!(tree.get(root_id).first_child, Some(child2_id));
        assert_eq!(tree.get(root_id).last_child, Some(child3_id));

        verify_tree_invariants(&tree);
    }

    // Tests for Tree::set_node (public within module)

    #[test]
    fn test_tree_set_node_data_only() {
        let mut tree: Tree<u16> = Tree {
            nodes: Vec::new(),
            levels: Vec::new(),
        };

        // Insert placeholder node
        let node_id = tree.insert_sorted(0, None);

        // Update data without reparenting
        tree.set_node_data(node_id, 1);

        // Verify data updated
        assert_eq!(tree.get(node_id).data, 1);

        // Verify level unchanged
        assert_eq!(tree.get(node_id).level, 0);

        verify_tree_invariants(&tree);
    }

    fn insert_tree(
        tree: &mut Tree<u16>,
        start_id: u16,
        node_count: usize,
        max_depth: usize,
        max_siblings: usize,
    ) {
        // Insert a deterministic sub-tree, filling level first
        let mut sibling_at_level: Vec<usize> = vec![];
        let mut parents: Vec<NodeId> = vec![];
        let mut inserted_count = 0;
        let mut next_id = start_id;
        loop {
            loop {
                let level = parents.len();
                let siblings_sum = if let Some(sum) = sibling_at_level.get_mut(level) {
                    *sum += 1;
                    *sum
                } else {
                    sibling_at_level.push(1);
                    1
                };
                if siblings_sum > max_siblings {
                    parents.pop();
                    sibling_at_level.pop();
                    if parents.is_empty() {
                        break;
                    }
                } else {
                    break;
                }
            }
            if sibling_at_level.is_empty() && parents.is_empty() {
                break;
            }
            if inserted_count >= node_count {
                break;
            }
            let parent_id = parents.last().cloned();

            let node_id = tree.insert_sorted(next_id, parent_id);
            inserted_count += 1;
            if parents.len() < max_depth {
                parents.push(node_id);
            }
            next_id += 1;
        }
    }

    #[test]
    fn test_tree_reparenting_single_node() {
        let mut tree: Tree<u16> = Tree {
            nodes: Vec::new(),
            levels: Vec::new(),
        };

        insert_tree(&mut tree, 0, 4, 1, 3);
        let root_id = NodeId(0);
        let child_id = NodeId(2);

        // Insert placeholder node
        let node_id = tree.insert_sorted(4, None);

        // Reparent under root
        tree.reparent_sorted(node_id, root_id);
        verify_tree_invariants(&tree);

        // Re-Reparent under child
        tree.reparent_sorted(node_id, child_id);
        verify_tree_invariants(&tree);
    }

    fn reparenting_subtrees(subtree_insertion: impl FnOnce(&mut Tree<u16>) -> NodeId) {
        let mut tree: Tree<u16> = Tree {
            nodes: Vec::new(),
            levels: Vec::new(),
        };

        insert_tree(&mut tree, 0, 6, 2, 3);
        let root_id = NodeId(0);
        let child_id = NodeId(2);

        // Insert another tree
        let tree_head_id = subtree_insertion(&mut tree);

        // Reparent under root
        tree.reparent_sorted(tree_head_id, root_id);
        verify_tree_invariants(&tree);

        // Re-Reparent under child
        tree.reparent_sorted(tree_head_id, child_id);
        verify_tree_invariants(&tree);
    }

    #[test]
    fn test_tree_reparenting_subtrees() {
        reparenting_subtrees(|tree| {
            insert_tree(tree, 6, 4, 1, 3);
            NodeId(6)
        });
    }

    #[test]
    fn test_tree_reparenting_bigger_subtree() {
        reparenting_subtrees(|tree| {
            insert_tree(tree, 6, 8, 3, 3);
            NodeId(6)
        });
    }

    // Tests for DocumentBuilderHeaderTree::add

    #[test]
    fn test_document_builder_header_tree_add_root_span() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add root span (no parent)
        let header = create_test_header(1, None, 1, 1.0, 2.0);
        let result = tree.add(header);

        assert!(result.is_ok());

        // Verify node added to index
        assert!(tree
            .node_index
            .get(&(TraceId::from(1), Id::from(1)))
            .is_some());

        // Verify tree structure
        assert_eq!(tree.header_tree.nodes.len(), 1);
        assert_eq!(tree.header_tree.levels.len(), 1);

        verify_tree_invariants(&tree.header_tree);
    }

    #[test]
    fn test_document_builder_header_tree_add_with_known_parent() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add parent span
        let parent = create_test_header(1, None, 1, 1.0, 2.0);
        tree.add(parent).unwrap();

        // Add child span with known parent
        let child = create_test_header(2, Some(1), 1, 2.0, 3.0);
        let result = tree.add(child);

        assert!(result.is_ok());

        // Verify both nodes in index
        assert!(tree
            .node_index
            .get(&(TraceId::from(1), Id::from(1)))
            .is_some());
        assert!(tree
            .node_index
            .get(&(TraceId::from(1), Id::from(2)))
            .is_some());

        // Verify tree structure
        assert_eq!(tree.header_tree.nodes.len(), 2);
        assert_eq!(tree.header_tree.levels.len(), 2);

        verify_tree_invariants(&tree.header_tree);
    }

    #[test]
    fn test_document_builder_header_tree_add_with_unknown_parent() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add child span before parent (creates placeholder)
        let child = create_test_header(2, Some(1), 1, 2.0, 3.0);
        let result = tree.add(child);

        assert!(result.is_ok());

        // Verify placeholder created for parent
        let parent_node_id = tree.node_index.get(&(TraceId::from(1), Id::from(1)));
        assert!(parent_node_id.is_some());

        // Verify placeholder has None data
        let parent_node = tree.header_tree.get(*parent_node_id.unwrap());
        assert!(parent_node.data.0.is_none());

        // Verify child node has Some data
        let child_node_id = tree.node_index.get(&(TraceId::from(1), Id::from(2)));
        let child_node = tree.header_tree.get(*child_node_id.unwrap());
        assert!(child_node.data.0.is_some());

        verify_tree_invariants(&tree.header_tree);
    }

    #[test]
    fn test_document_builder_header_tree_add_fill_placeholder() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add child span (creates placeholder for parent)
        let child = create_test_header(2, Some(1), 1, 2.0, 3.0);
        tree.add(child).unwrap();

        // Add parent span (fills placeholder)
        let parent = create_test_header(1, None, 1, 1.0, 2.0);
        let result = tree.add(parent);

        assert!(result.is_ok());

        // Verify placeholder filled
        let parent_node_id = tree
            .node_index
            .get(&(TraceId::from(1), Id::from(1)))
            .unwrap();
        let parent_node = tree.header_tree.get(*parent_node_id);
        assert!(parent_node.data.0.is_some());
        assert_eq!(parent_node.data.0.as_ref().unwrap().id, Some(Id::from(1)));

        verify_tree_invariants(&tree.header_tree);
    }

    #[test]
    fn test_document_builder_header_tree_add_fill_placeholder_with_reparent() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add child before parent (creates placeholder for parent)
        let child = create_test_header(2, Some(1), 1, 2.0, 3.0);
        tree.add(child).unwrap();

        // Verify placeholder created for parent
        let parent_placeholder = tree.node_index.get(&(TraceId::from(1), Id::from(1)));
        assert!(parent_placeholder.is_some());

        // Add parent (fills placeholder)
        let parent = create_test_header(1, None, 1, 1.0, 2.0);
        let result = tree.add(parent);

        assert!(result.is_ok());

        // Verify parent placeholder filled
        let parent_node_id = tree
            .node_index
            .get(&(TraceId::from(1), Id::from(1)))
            .unwrap();
        let parent_node = tree.header_tree.get(*parent_node_id);
        assert!(parent_node.data.0.is_some());
        assert_eq!(parent_node.data.0.as_ref().unwrap().id, Some(Id::from(1)));

        // Verify child is still present
        let child_node_id = tree.node_index.get(&(TraceId::from(1), Id::from(2)));
        assert!(child_node_id.is_some());

        verify_tree_invariants(&tree.header_tree);
    }

    #[test]
    fn test_document_builder_header_tree_add_missing_id() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Create header with missing id
        let header = DocumentBuilderHeader {
            id: None,
            parent_id: None,
            trace_id: Some(TraceId::from(1)),
            start_time: Some(1.0),
            end_time: Some(2.0),
        };

        let result = tree.add(header);

        // Result is TranslationError which wraps ConstraintError
        assert!(result.is_err());
    }

    #[test]
    fn test_document_builder_header_tree_add_missing_trace_id() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Create header with missing trace_id
        let header = DocumentBuilderHeader {
            id: Some(Id::from(1)),
            parent_id: None,
            trace_id: None,
            start_time: Some(1.0),
            end_time: Some(2.0),
        };

        let result = tree.add(header);

        // Result is TranslationError which wraps ConstraintError
        assert!(result.is_err());
    }

    // Tests for DocumentBuilderHeaderTreeIterator::next

    #[test]
    fn test_iterator_empty_tree() {
        let tree = DocumentBuilderHeaderTree::new(10);

        // Empty tree has no levels, so we can't create an iterator
        // The iter() method will underflow when computing current_level
        // This is expected behavior - you shouldn't iterate an empty tree
        // Just verify the tree is empty
        assert_eq!(tree.header_tree.nodes.len(), 0);
        assert_eq!(tree.header_tree.levels.len(), 0);
    }

    #[test]
    fn test_iterator_single_root() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add single root node
        let header = create_test_header(1, None, 1, 1.0, 2.0);
        tree.add(header).unwrap();

        let mut iter = tree.iter();

        // Should yield the single root node
        let result = iter.next();
        assert!(result.is_some());
        assert_eq!(result.unwrap().id, Some(Id::from(1)));

        // Should be done
        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iterator_multiple_roots() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add multiple root nodes with different start times
        tree.add(create_test_header(1, None, 1, 2.0, 3.0)).unwrap();
        tree.add(create_test_header(2, None, 1, 1.0, 2.0)).unwrap();
        tree.add(create_test_header(3, None, 1, 3.0, 4.0)).unwrap();

        let mut iter = tree.iter();

        // Roots are not sorted - they're yielded in insertion order: 1, 2, 3
        // (Roots have no parent, so SiblingOrd doesn't apply to them)
        let first = iter.next().unwrap();
        assert_eq!(first.id, Some(Id::from(1)));

        let second = iter.next().unwrap();
        assert_eq!(second.id, Some(Id::from(2)));

        let third = iter.next().unwrap();
        assert_eq!(third.id, Some(Id::from(3)));

        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iterator_three_levels_bottom_up() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add root
        tree.add(create_test_header(1, None, 1, 1.0, 2.0)).unwrap();

        // Add child
        tree.add(create_test_header(2, Some(1), 1, 2.0, 3.0))
            .unwrap();

        // Add grandchild
        tree.add(create_test_header(3, Some(2), 1, 3.0, 4.0))
            .unwrap();

        let mut iter = tree.iter();

        // Should yield bottom-up: grandchild (3), child (2), root (1)
        let first = iter.next().unwrap();
        assert_eq!(first.id, Some(Id::from(3)));

        let second = iter.next().unwrap();
        assert_eq!(second.id, Some(Id::from(2)));

        let third = iter.next().unwrap();
        assert_eq!(third.id, Some(Id::from(1)));

        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iterator_siblings_chronological_order() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add root
        tree.add(create_test_header(1, None, 1, 1.0, 2.0)).unwrap();

        // Add children with different start times (out of order)
        tree.add(create_test_header(2, Some(1), 1, 3.0, 4.0))
            .unwrap();
        tree.add(create_test_header(3, Some(1), 1, 2.0, 3.0))
            .unwrap();
        tree.add(create_test_header(4, Some(1), 1, 4.0, 5.0))
            .unwrap();

        let mut iter = tree.iter();

        // Should yield children in chronological order: 3 (2.0), 2 (3.0), 4 (4.0), then root 1
        let first = iter.next().unwrap();
        assert_eq!(first.id, Some(Id::from(3)));

        let second = iter.next().unwrap();
        assert_eq!(second.id, Some(Id::from(2)));

        let third = iter.next().unwrap();
        assert_eq!(third.id, Some(Id::from(4)));

        let fourth = iter.next().unwrap();
        assert_eq!(fourth.id, Some(Id::from(1)));

        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iterator_skips_placeholders() {
        let mut tree = DocumentBuilderHeaderTree::new(10);

        // Add child (creates placeholder for parent)
        tree.add(create_test_header(2, Some(1), 1, 2.0, 3.0))
            .unwrap();

        let mut iter = tree.iter();

        // Should yield only the child, skipping the placeholder parent
        let first = iter.next().unwrap();
        assert_eq!(first.id, Some(Id::from(2)));

        assert!(iter.next().is_none());
    }

    #[test]
    fn test_iterator_complex_tree() {
        // Build 8 headers that will result in the following tree:
        // Root 1 (1.0 -> 5.0)
        //   Child 2 (1.0 -> 2.0)
        //   Child 3 (2.0 -> 4.0)
        //     Grandchild 4 (2.1 -> 3.0)
        //     Grandchild 5 (3.0 -> 3.9)
        //   Child 6 (4.0 -> 5.0)
        //     Grandchild 7 (4.1 -> 4.9)
        // Root 8 (4.0 -> 4.5)
        //
        // No matter which of the 40320 possible order of insertion we use,
        // we should always get a "correct" iterator in the end
        // (that's the entire purpose of this data structure)
        //
        // A correct iterator will yield:
        //  - All Grandchildren first
        //  - Grandchild 4 before Grandchild 5
        //  - Then all chidren
        //  - In the exact 2 -> 3 -> 6 order
        //  - Roots, in whatever order

        let headers = [
            create_test_header(1, None, 1, 1.0, 5.0),
            create_test_header(2, Some(1), 1, 1.0, 2.0),
            create_test_header(3, Some(1), 1, 2.0, 4.0),
            create_test_header(4, Some(3), 1, 2.1, 3.0),
            create_test_header(5, Some(3), 1, 3.0, 3.9),
            create_test_header(6, Some(1), 1, 4.0, 5.0),
            create_test_header(7, Some(6), 1, 4.1, 4.9),
            create_test_header(8, None, 2, 4.0, 4.5),
        ];

        struct CombinationIterator {
            picked: [usize; 8],
            availables: [bool; 8],
            end_reached: bool,
        }
        impl CombinationIterator {
            fn new() -> Self {
                Self {
                    picked: [0, 1, 2, 3, 4, 5, 6, 7],
                    availables: [false; 8],
                    end_reached: false,
                }
            }
        }
        impl Iterator for CombinationIterator {
            type Item = [usize; 8];

            fn next(&mut self) -> Option<Self::Item> {
                let Self {
                    picked,
                    availables,
                    end_reached,
                } = self;
                if *end_reached {
                    None
                } else {
                    let next = Some(*picked);
                    // Start from last elem
                    let mut cursor = picked.len() - 1;
                    let made_progress = loop {
                        let p = picked[cursor];
                        // We are looking for an available index that is strictly greater than the current picked one
                        if let Some((index, _)) =
                            availables.iter().enumerate().skip(p + 1).find(|(_, b)| **b)
                        {
                            availables[p] = true;
                            picked[cursor] = index;
                            availables[index] = false;
                            cursor += 1;
                            // We made progress
                            break true;
                        } else {
                            // There is no such index:
                            // free the one we are using and
                            // look at the previous pick
                            availables[p] = true;
                            if cursor > 0 {
                                cursor -= 1;
                            } else {
                                break false;
                            }
                        }
                    };
                    if made_progress {
                        // Fill with availables
                        while cursor < picked.len() {
                            let index = availables.iter().position(|b| *b).unwrap();
                            picked[cursor] = index;
                            availables[index] = false;
                            cursor += 1;
                        }
                    } else {
                        *end_reached = true;
                    }

                    next
                }
            }
        }

        let mut test_count = 0usize;
        // Loop through all the insertion orders
        for indexes in CombinationIterator::new() {
            test_count += 1;
            // Create tree
            let mut tree = DocumentBuilderHeaderTree::new(8);
            for i in indexes {
                tree.add(headers[i]).unwrap();
            }

            // Test iterator
            let mut expect = None;
            let mut expect_grand_children = true;
            let mut expect_root = false;
            let mut count = 0usize;
            fn is_grand_child(header: DocumentBuilderHeader) -> bool {
                [Id::from(4), Id::from(5), Id::from(7)].contains(&header.id.unwrap())
            }
            fn is_root(header: DocumentBuilderHeader) -> bool {
                [Id::from(1), Id::from(8)].contains(&header.id.unwrap())
            }
            fn is_id(header: DocumentBuilderHeader, id: u64) -> bool {
                header.id == Some(Id::from(id))
            }
            fn assert_id(header: DocumentBuilderHeader, id: u64) {
                assert!(is_id(header, id), "Should have been Id({id})");
            }

            for header in tree.iter() {
                // Make assertions
                assert!(
                    !expect_grand_children || is_grand_child(header),
                    "Should have been a grandchild"
                );
                assert!(
                    !expect_root || is_root(header),
                    "Should have been a grandchild"
                );
                if let Some(id) = expect {
                    assert_id(header, id);
                }
                // Prepare next iteration
                count += 1;
                if count == 6 {
                    expect_root = true;
                }

                expect = if is_id(header, 4) {
                    Some(5)
                } else if is_id(header, 2) {
                    Some(3)
                } else if is_id(header, 3) {
                    Some(6)
                } else if count == 3 {
                    expect_grand_children = false;
                    Some(2)
                } else {
                    None
                };
            }
            assert_eq!(count, 8, "Iterator did not yield all the nodes");
        }
        dbg!(test_count);
    }
}
