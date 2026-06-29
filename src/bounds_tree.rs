use crate::{Bounds, Half};
use std::{
    cmp,
    fmt::Debug,
    ops::{Add, Sub},
};

#[derive(Debug)]
pub(crate) struct BoundsTree<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    root: Option<usize>,
    nodes: Vec<Node<U>>,
    stack: Vec<usize>,
    /// Minimum ordering assigned to any subsequent insert. Raised before painting deferred
    /// draws so overlays always sort above the main scene (and their orders can't fall inside a
    /// content-filter order range from the main scene). 0 means no floor.
    order_floor: u32,
}

impl<U> BoundsTree<U>
where
    U: Clone
        + Debug
        + PartialEq
        + PartialOrd
        + Add<U, Output = U>
        + Sub<Output = U>
        + Half
        + Default,
{
    pub fn clear(&mut self) {
        self.root = None;
        self.nodes.clear();
        self.stack.clear();
        self.order_floor = 0;
    }

    /// Raise the minimum ordering for subsequent inserts to `floor`. Relative ordering above the
    /// floor is preserved (overlapping inserts still step above one another).
    pub fn set_order_floor(&mut self, floor: u32) {
        self.order_floor = self.order_floor.max(floor);
    }

    /// The highest ordering assigned to any bounds so far (0 if empty).
    pub fn max_order(&self) -> u32 {
        self.root.map_or(0, |index| self.nodes[index].max_ordering())
    }

    /// Inserts bounds with an ordering strictly greater than *every* existing bounds (not just
    /// intersecting ones), and returns that ordering. Used for content-filter group boundaries
    /// (which must sort after all previously-painted content so their order range can't collide
    /// with unrelated non-overlapping content that reuses low orderings) and to raise the order
    /// floor before painting deferred draws (so overlays always sort above the main scene).
    pub fn insert_above_all(&mut self, new_bounds: Bounds<U>) -> u32 {
        let ordering = self.max_order() + 1;
        self.insert_at_order(new_bounds, ordering)
    }

    pub fn insert(&mut self, new_bounds: Bounds<U>) -> u32 {
        // If the tree is empty, make the root the new leaf.
        let Some(mut index) = self.root else {
            let ordering = 1u32.max(self.order_floor);
            let new_node = self.push_leaf(new_bounds, ordering);
            self.root = Some(new_node);
            return ordering;
        };

        // Search for the best place to add the new leaf based on heuristics.
        let mut max_intersecting_ordering = 0;
        while let Node::Internal {
            left,
            right,
            bounds: node_bounds,
            ..
        } = &mut self.nodes[index]
        {
            let left = *left;
            let right = *right;
            *node_bounds = node_bounds.union(&new_bounds);
            self.stack.push(index);

            // Descend to the best-fit child, based on which one would increase
            // the surface area the least. This attempts to keep the tree balanced
            // in terms of surface area. If there is an intersection with the other child,
            // add its keys to the intersections vector.
            let left_cost = new_bounds.union(self.nodes[left].bounds()).half_perimeter();
            let right_cost = new_bounds
                .union(self.nodes[right].bounds())
                .half_perimeter();
            if left_cost < right_cost {
                max_intersecting_ordering =
                    self.find_max_ordering(right, &new_bounds, max_intersecting_ordering);
                index = left;
            } else {
                max_intersecting_ordering =
                    self.find_max_ordering(left, &new_bounds, max_intersecting_ordering);
                index = right;
            }
        }

        // We've found a leaf ('index' now refers to a leaf node).
        // We'll insert a new parent node above the leaf and attach our new leaf to it.
        let sibling = index;

        // Check for collision with the located leaf node
        let Node::Leaf {
            bounds: sibling_bounds,
            order: sibling_ordering,
            ..
        } = &self.nodes[index]
        else {
            unreachable!();
        };
        if sibling_bounds.intersects(&new_bounds) {
            max_intersecting_ordering = cmp::max(max_intersecting_ordering, *sibling_ordering);
        }

        let ordering = (max_intersecting_ordering + 1).max(self.order_floor);
        self.attach_leaf(sibling, new_bounds, ordering)
    }

    /// Inserts `new_bounds` as a leaf at exactly `ordering`, ignoring intersections (the caller
    /// guarantees `ordering` is already greater than any ordering it could collide with). Used by
    /// [`Self::insert_above_all`], which only needs to find an attachment point in the tree.
    fn insert_at_order(&mut self, new_bounds: Bounds<U>, ordering: u32) -> u32 {
        let Some(mut index) = self.root else {
            let new_node = self.push_leaf(new_bounds, ordering);
            self.root = Some(new_node);
            return ordering;
        };

        while let Node::Internal {
            left,
            right,
            bounds: node_bounds,
            ..
        } = &mut self.nodes[index]
        {
            let left = *left;
            let right = *right;
            *node_bounds = node_bounds.union(&new_bounds);
            self.stack.push(index);

            let left_cost = new_bounds.union(self.nodes[left].bounds()).half_perimeter();
            let right_cost = new_bounds
                .union(self.nodes[right].bounds())
                .half_perimeter();
            if left_cost < right_cost {
                index = left;
            } else {
                index = right;
            }
        }

        let sibling = index;
        self.attach_leaf(sibling, new_bounds, ordering)
    }

    /// Attaches a new leaf with `bounds`/`ordering` as a sibling of the leaf at `sibling`,
    /// rewiring the parent chain (tracked in `self.stack`) and propagating `max_order` upward.
    fn attach_leaf(&mut self, sibling: usize, bounds: Bounds<U>, ordering: u32) -> u32 {
        let new_node = self.push_leaf(bounds, ordering);
        let new_parent = self.push_internal(sibling, new_node);

        // If there was an old parent, we need to update its children indices.
        if let Some(old_parent) = self.stack.last().copied() {
            let Node::Internal { left, right, .. } = &mut self.nodes[old_parent] else {
                unreachable!();
            };

            if *left == sibling {
                *left = new_parent;
            } else {
                *right = new_parent;
            }
        } else {
            // If the old parent was the root, the new parent is the new root.
            self.root = Some(new_parent);
        }

        for node_index in self.stack.drain(..).rev() {
            let Node::Internal {
                max_order: max_ordering,
                ..
            } = &mut self.nodes[node_index]
            else {
                unreachable!()
            };
            if *max_ordering >= ordering {
                break;
            }
            *max_ordering = ordering;
        }

        ordering
    }

    fn find_max_ordering(&self, index: usize, bounds: &Bounds<U>, mut max_ordering: u32) -> u32 {
        match &self.nodes[index] {
            Node::Leaf {
                bounds: node_bounds,
                order: ordering,
                ..
            } => {
                if bounds.intersects(node_bounds) {
                    max_ordering = cmp::max(*ordering, max_ordering);
                }
            }
            Node::Internal {
                left,
                right,
                bounds: node_bounds,
                max_order: node_max_ordering,
                ..
            } => {
                if bounds.intersects(node_bounds) && max_ordering < *node_max_ordering {
                    let left_max_ordering = self.nodes[*left].max_ordering();
                    let right_max_ordering = self.nodes[*right].max_ordering();
                    if left_max_ordering > right_max_ordering {
                        max_ordering = self.find_max_ordering(*left, bounds, max_ordering);
                        max_ordering = self.find_max_ordering(*right, bounds, max_ordering);
                    } else {
                        max_ordering = self.find_max_ordering(*right, bounds, max_ordering);
                        max_ordering = self.find_max_ordering(*left, bounds, max_ordering);
                    }
                }
            }
        }
        max_ordering
    }

    fn push_leaf(&mut self, bounds: Bounds<U>, order: u32) -> usize {
        self.nodes.push(Node::Leaf { bounds, order });
        self.nodes.len() - 1
    }

    fn push_internal(&mut self, left: usize, right: usize) -> usize {
        let left_node = &self.nodes[left];
        let right_node = &self.nodes[right];
        let new_bounds = left_node.bounds().union(right_node.bounds());
        let max_ordering = cmp::max(left_node.max_ordering(), right_node.max_ordering());
        self.nodes.push(Node::Internal {
            bounds: new_bounds,
            left,
            right,
            max_order: max_ordering,
        });
        self.nodes.len() - 1
    }
}

impl<U> Default for BoundsTree<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    fn default() -> Self {
        BoundsTree {
            root: None,
            nodes: Vec::new(),
            stack: Vec::new(),
            order_floor: 0,
        }
    }
}

#[derive(Debug, Clone)]
enum Node<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    Leaf {
        bounds: Bounds<U>,
        order: u32,
    },
    Internal {
        left: usize,
        right: usize,
        bounds: Bounds<U>,
        max_order: u32,
    },
}

impl<U> Node<U>
where
    U: Clone + Debug + Default + PartialEq,
{
    fn bounds(&self) -> &Bounds<U> {
        match self {
            Node::Leaf { bounds, .. } => bounds,
            Node::Internal { bounds, .. } => bounds,
        }
    }

    fn max_ordering(&self) -> u32 {
        match self {
            Node::Leaf {
                order: ordering, ..
            } => *ordering,
            Node::Internal {
                max_order: max_ordering,
                ..
            } => *max_ordering,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Bounds, Point, Size};
    use rand::{RngExt, SeedableRng};

    #[test]
    fn test_insert() {
        let mut tree = BoundsTree::<f32>::default();
        let bounds1 = Bounds {
            origin: Point { x: 0.0, y: 0.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds2 = Bounds {
            origin: Point { x: 5.0, y: 5.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds3 = Bounds {
            origin: Point { x: 10.0, y: 10.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };

        // Insert the bounds into the tree and verify the order is correct
        assert_eq!(tree.insert(bounds1), 1);
        assert_eq!(tree.insert(bounds2), 2);
        assert_eq!(tree.insert(bounds3), 3);

        // Insert non-overlapping bounds and verify they can reuse orders
        let bounds4 = Bounds {
            origin: Point { x: 20.0, y: 20.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds5 = Bounds {
            origin: Point { x: 40.0, y: 40.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        let bounds6 = Bounds {
            origin: Point { x: 25.0, y: 25.0 },
            size: Size {
                width: 10.0,
                height: 10.0,
            },
        };
        assert_eq!(tree.insert(bounds4), 1); // bounds4 does not overlap with bounds1, bounds2, or bounds3
        assert_eq!(tree.insert(bounds5), 1); // bounds5 does not overlap with any other bounds
        assert_eq!(tree.insert(bounds6), 2); // bounds6 overlaps with bounds4, so it should have a different order
    }

    #[test]
    fn test_random_iterations() {
        let max_bounds = 100;
        for seed in 1..=1000 {
            // let seed = 44;
            let mut tree = BoundsTree::default();
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed as u64);
            let mut expected_quads: Vec<(Bounds<f32>, u32)> = Vec::new();

            // Insert a random number of random AABBs into the tree.
            let num_bounds = rng.random_range(1..=max_bounds);
            for _ in 0..num_bounds {
                let min_x: f32 = rng.random_range(-100.0..100.0);
                let min_y: f32 = rng.random_range(-100.0..100.0);
                let width: f32 = rng.random_range(0.0..50.0);
                let height: f32 = rng.random_range(0.0..50.0);
                let bounds = Bounds {
                    origin: Point { x: min_x, y: min_y },
                    size: Size { width, height },
                };

                let expected_ordering = expected_quads
                    .iter()
                    .filter_map(|quad| quad.0.intersects(&bounds).then_some(quad.1))
                    .max()
                    .unwrap_or(0)
                    + 1;
                expected_quads.push((bounds, expected_ordering));

                // Insert the AABB into the tree and collect intersections.
                let actual_ordering = tree.insert(bounds);
                assert_eq!(actual_ordering, expected_ordering);
            }
        }
    }
}
