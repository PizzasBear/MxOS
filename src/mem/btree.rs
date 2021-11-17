// #![allow(dead_code)]

// TODO: Optimize the number of moves (and copies potentialy?) of StackVec based structs.

use crate::{
    mem::{SlabAllocator, SlabBox},
    ref_stack::OnStackRefMutStack,
    stack_vec::{OuterLenStackVec, StackVec, StackVecIntoIter},
};
use core::borrow::Borrow;
use core::cmp::Ordering;
use core::{fmt, mem, ops, ptr, slice};

// use core::marker::PhantomData;

const B: usize = 6;

const MIN_NUM_ELEMENTS: usize = B - 1;
const MAX_NUM_ELEMENTS: usize = 2 * B - 1;
const MIN_NUM_CHILDREN: usize = B;
const MAX_NUM_CHILDREN: usize = 2 * B;

trait OptionExt {
    fn assert_none(&self);
}

impl<T> OptionExt for Option<T> {
    #[inline]
    fn assert_none(&self) {
        assert!(
            self.is_none(),
            "called `Option::unwrap()` on a `None` value",
        );
    }
}

trait BoundClonedExt {
    type Target: Clone;

    fn cloned(&self) -> ops::Bound<Self::Target>;
}

impl<T: Clone> BoundClonedExt for ops::Bound<&T> {
    type Target = T;

    fn cloned(&self) -> ops::Bound<T> {
        match self {
            Self::Unbounded => ops::Bound::Unbounded,
            Self::Included(x) => ops::Bound::Included((*x).clone()),
            Self::Excluded(x) => ops::Bound::Excluded((*x).clone()),
        }
    }
}

#[repr(C)]
struct NodeElements<K: Ord, V> {
    _keys: OuterLenStackVec<K, MAX_NUM_ELEMENTS>,
    _values: OuterLenStackVec<V, MAX_NUM_ELEMENTS>,

    _len: u8,
    // parent: *mut Node<K, V>,
}

impl<K: Ord, V> NodeElements<K, V> {
    // pub fn new(parent: *mut Node<K, V>) -> Self {
    //     unsafe { Self::from_raw_parts(OuterLenStackVec::new(), OuterLenStackVec::new(), 0, parent) }
    // }
    pub fn new() -> Self {
        unsafe { Self::from_raw_parts(OuterLenStackVec::new(), OuterLenStackVec::new(), 0) }
    }

    #[inline]
    pub fn keys(&self) -> &[K] {
        unsafe { self._keys.as_slice(self.len()) }
    }

    #[inline]
    pub fn keys_mut(&mut self) -> &mut [K] {
        unsafe { self._keys.as_slice_mut(self.len()) }
    }

    #[inline]
    pub fn values(&self) -> &[V] {
        unsafe { self._values.as_slice(self.len()) }
    }

    #[inline]
    pub fn values_mut(&mut self) -> &mut [V] {
        unsafe { self._values.as_slice_mut(self.len()) }
    }

    /// Get both `keys` and `values` as mutables at the same.
    #[inline]
    pub fn get_all_mut(&mut self) -> (&mut [K], &mut [V]) {
        let len = self.len();
        unsafe { (self._keys.as_slice_mut(len), self._values.as_slice_mut(len)) }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self._len as _
    }

    #[inline(always)]
    pub unsafe fn set_len(&mut self, len: usize) {
        self._len = len as _;
    }

    #[must_use]
    #[inline]
    pub fn push(&mut self, k: K, value: V) -> Option<(K, V)> {
        unsafe {
            let overflow_k = self._keys.push(&mut self.len(), k);
            let mut len = self.len();
            let overflow_value = self._values.push(&mut len, value);
            self.set_len(len);

            match (overflow_k, overflow_value) {
                (Some(overflow_k), Some(overflow_value)) => Some((overflow_k, overflow_value)),
                (None, None) => None,
                _ => unreachable!(),
            }
        }
    }

    #[must_use]
    pub fn insert(&mut self, idx: usize, k: K, value: V) -> Option<(K, V)> {
        unsafe {
            let mut len = self.len();

            let overflow_k = self._keys.insert(&mut len.clone(), idx, k);
            let overflow_value = self._values.insert(&mut len, idx, value);

            self.set_len(len);

            match (overflow_k, overflow_value) {
                (Some(overflow_k), Some(overflow_value)) => Some((overflow_k, overflow_value)),
                (None, None) => None,
                _ => unreachable!(),
            }
        }
    }

    #[inline]
    pub fn pop(&mut self) -> Option<(K, V)> {
        unsafe {
            let mut len = self.len();

            let popped_k = self._keys.pop(&mut len.clone());
            let popped_value = self._values.pop(&mut len);

            self.set_len(len);

            match (popped_k, popped_value) {
                (Some(popped_k), Some(popped_value)) => Some((popped_k, popped_value)),
                (None, None) => None,
                _ => unreachable!(),
            }
        }
    }

    pub fn remove(&mut self, idx: usize) -> (K, V) {
        unsafe {
            let mut len = self.len();

            let removed_k = self._keys.remove(&mut len.clone(), idx);
            let removed_value = self._values.remove(&mut len, idx);

            self.set_len(len);

            (removed_k, removed_value)
        }
    }

    #[inline]
    fn into_raw_parts(
        self,
    ) -> (
        OuterLenStackVec<K, MAX_NUM_ELEMENTS>,
        OuterLenStackVec<V, MAX_NUM_ELEMENTS>,
        usize,
        // *mut Node<K, V>,
    ) {
        unsafe {
            let mb = mem::ManuallyDrop::new(self);
            (
                ptr::read(&mb._keys),
                ptr::read(&mb._values),
                mb._len as _,
                // mb.parent,
            )
        }
    }

    #[inline]
    unsafe fn from_raw_parts(
        keys: OuterLenStackVec<K, MAX_NUM_ELEMENTS>,
        values: OuterLenStackVec<V, MAX_NUM_ELEMENTS>,
        len: usize,
        // parent: *mut Node<K, V>,
    ) -> Self {
        Self {
            _keys: keys,
            _values: values,
            _len: len as _,
            // parent,
        }
    }

    #[inline]
    pub fn separate(
        self,
    ) -> (
        StackVec<K, MAX_NUM_ELEMENTS>,
        StackVec<V, MAX_NUM_ELEMENTS>,
        // *mut Node<K, V>,
    ) {
        unsafe {
            // let (keys, values, len, parent) = self.into_raw_parts();
            let (keys, values, len) = self.into_raw_parts();

            (
                StackVec::from_raw_parts(keys, len),
                StackVec::from_raw_parts(values, len),
                // parent,
            )
        }
    }

    // #[inline]
    // pub fn split(&mut self, rightmost_key: K, rightmost_value: V) -> (K, V, Self) {
    //     assert_eq!(self.len(), MAX_NUM_ELEMENTS);
    //     unsafe {
    //         let mut len = self.len();

    //         let mut right_keys = self
    //             ._keys
    //             .split_at(&mut len.clone(), MAX_NUM_ELEMENTS / 2 + 1)
    //             .into_raw_parts()
    //             .0;
    //         let (mut right_values, mut right_len) = self
    //             ._values
    //             .split_at(&mut len, MAX_NUM_ELEMENTS / 2 + 1)
    //             .into_raw_parts();

    //         right_keys
    //             .push(&mut right_len.clone(), rightmost_key)
    //             .assert_none();
    //         right_values
    //             .push(&mut right_len, rightmost_value)
    //             .assert_none();

    //         let sep_k = self._keys.pop(&mut len.clone()).unwrap();
    //         let sep_value = self._values.pop(&mut len).unwrap();

    //         self.set_len(len);

    //         (
    //             sep_k,
    //             sep_value,
    //             Self::from_raw_parts(right_keys, right_values, right_len),
    //         )
    //     }
    // }

    #[inline]
    pub fn split(&mut self, rightmost_key: K, rightmost_value: V, right: &mut Self) -> (K, V) {
        assert_eq!(self.len(), MAX_NUM_ELEMENTS);
        assert_eq!(right.len(), 0);

        unsafe {
            ptr::copy_nonoverlapping(
                self._keys.as_ptr().add(B + 1),
                right._keys.as_mut_ptr(),
                B - 2,
            );
            ptr::copy_nonoverlapping(
                self._values.as_ptr().add(B + 1),
                right._values.as_mut_ptr(),
                B - 2,
            );
            right.set_len(B - 2);
            right.push(rightmost_key, rightmost_value).assert_none();

            self.set_len(B + 1);
            self.pop().unwrap()
        }
    }

    #[inline]
    pub fn merge(&mut self, sep_k: K, sep_value: V, right: &mut Self) {
        assert!(self.len() + right.len() < MAX_NUM_ELEMENTS);
        unsafe {
            self.push(sep_k, sep_value).assert_none();
            ptr::copy_nonoverlapping(
                right._keys.as_ptr(),
                self._keys.as_mut_ptr().add(self.len()),
                right.len(),
            );
            ptr::copy_nonoverlapping(
                right._values.as_ptr(),
                self._values.as_mut_ptr().add(self.len()),
                right.len(),
            );
            self.set_len(self.len() + right.len());
            right.set_len(0);
        }
    }
}

impl<K: Ord, V> Default for NodeElements<K, V> {
    fn default() -> Self {
        // Self::new(ptr::null_mut())
        Self::new()
    }
}

impl<K: Ord + Clone, V: Clone> Clone for NodeElements<K, V> {
    fn clone(&self) -> Self {
        unsafe {
            Self::from_raw_parts(
                self._keys.clone(self.len()).into_raw_parts().0,
                self._values.clone(self.len()).into_raw_parts().0,
                self.len(),
                // self.parent,
            )
        }
    }
}

impl<K: Ord + fmt::Debug, V: fmt::Debug> fmt::Debug for NodeElements<K, V> {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeData")
            .field("keys", &self.keys())
            .field("values", &self.values())
            .finish()
    }
}

impl<K: Ord, V> Drop for NodeElements<K, V> {
    fn drop(&mut self) {
        while let Some(_) = self.pop() {}
    }
}

#[repr(C)]
struct Children<K: Ord, V> {
    _data: OuterLenChildren<K, V>,
    _len: usize,
}

enum ChildrenStackVec<K: Ord, V> {
    Nodes(StackVec<SlabBox<Node<K, V>>, MAX_NUM_CHILDREN>),
    Leafs(StackVec<SlabBox<NodeElements<K, V>>, MAX_NUM_CHILDREN>),
}

#[repr(u8)]
enum OuterLenChildren<K: Ord, V> {
    Nodes(OuterLenStackVec<SlabBox<Node<K, V>>, MAX_NUM_CHILDREN>),
    Leafs(OuterLenStackVec<SlabBox<NodeElements<K, V>>, MAX_NUM_CHILDREN>),
}

// #[derive(Debug, Clone)]
#[derive(Debug)]
enum Child<K: Ord, V> {
    Node(SlabBox<Node<K, V>>),
    Leaf(SlabBox<NodeElements<K, V>>),
}

#[derive(Debug)]
enum ChildRef<'a, K: Ord, V> {
    Node(&'a Node<K, V>),
    Leaf(&'a NodeElements<K, V>),
}

#[derive(Debug)]
enum ChildRefMut<'a, K: Ord, V> {
    Node(&'a mut Node<K, V>),
    Leaf(&'a mut NodeElements<K, V>),
}

#[derive(Debug)]
enum ChildPtrMut<K: Ord, V> {
    Node(*mut Node<K, V>),
    Leaf(*mut NodeElements<K, V>),
}

#[derive(Debug)]
enum ChildrenSlice<'a, K: Ord, V> {
    Nodes(&'a [SlabBox<Node<K, V>>]),
    Leafs(&'a [SlabBox<NodeElements<K, V>>]),
}

#[derive(Debug)]
enum ChildrenSliceMut<'a, K: Ord, V> {
    Nodes(&'a mut [SlabBox<Node<K, V>>]),
    Leafs(&'a mut [SlabBox<NodeElements<K, V>>]),
}

#[derive(Debug)]
enum ChildrenIter<'a, K: Ord, V> {
    Nodes(slice::Iter<'a, SlabBox<Node<K, V>>>),
    Leafs(slice::Iter<'a, SlabBox<NodeElements<K, V>>>),
}

#[derive(Debug)]
enum ChildrenIterMut<'a, K: Ord, V> {
    Nodes(slice::IterMut<'a, SlabBox<Node<K, V>>>),
    Leafs(slice::IterMut<'a, SlabBox<NodeElements<K, V>>>),
}

// #[derive(Debug, Clone)]
#[derive(Debug)]
enum ChildrenIntoIter<K: Ord, V> {
    Nodes(StackVecIntoIter<SlabBox<Node<K, V>>, MAX_NUM_CHILDREN>),
    Leafs(StackVecIntoIter<SlabBox<NodeElements<K, V>>, MAX_NUM_CHILDREN>),
}

impl<K: Ord, V> Child<K, V> {
    pub fn num_elements(&self) -> usize {
        match self {
            Self::Node(node) => node.num_elements(),
            Self::Leaf(leaf) => leaf.len(),
        }
    }

    // pub fn parent(&self) -> *const Node<K, V> {
    //     match self {
    //         Self::Node(node) => node.parent(),
    //         Self::Leaf(leaf) => leaf.parent,
    //     }
    // }

    // pub fn parent_mut(&mut self) -> &mut *mut Node<K, V> {
    //     match self {
    //         Self::Node(node) => node.parent_mut(),
    //         Self::Leaf(leaf) => &mut leaf.parent,
    //     }
    // }

    pub fn as_ref(&self) -> ChildRef<K, V> {
        match self {
            Self::Node(node) => ChildRef::Node(node),
            Self::Leaf(leaf) => ChildRef::Leaf(leaf),
        }
    }

    pub fn as_mut(&mut self) -> ChildRefMut<K, V> {
        match self {
            Self::Node(node) => ChildRefMut::Node(node),
            Self::Leaf(leaf) => ChildRefMut::Leaf(leaf),
        }
    }

    pub fn try_into_node(self) -> Option<SlabBox<Node<K, V>>> {
        match self {
            Self::Node(node) => Some(node),
            Self::Leaf(_) => None,
        }
    }

    pub fn try_into_leaf(self) -> Option<SlabBox<NodeElements<K, V>>> {
        match self {
            Self::Leaf(leaf) => Some(leaf),
            Self::Node(_) => None,
        }
    }

    pub fn try_as_node_ref(&self) -> Option<&SlabBox<Node<K, V>>> {
        match self {
            Self::Node(node) => Some(node),
            Self::Leaf(_) => None,
        }
    }

    pub fn try_as_leaf_ref(&self) -> Option<&SlabBox<NodeElements<K, V>>> {
        match self {
            Self::Leaf(leaf) => Some(leaf),
            Self::Node(_) => None,
        }
    }

    pub fn try_as_node_mut(&mut self) -> Option<&mut SlabBox<Node<K, V>>> {
        match self {
            Self::Node(node) => Some(node),
            Self::Leaf(_) => None,
        }
    }

    pub fn try_as_leaf_mut(&mut self) -> Option<&mut SlabBox<NodeElements<K, V>>> {
        match self {
            Self::Leaf(leaf) => Some(leaf),
            Self::Node(_) => None,
        }
    }

    pub fn keys(&self) -> &[K] {
        match self {
            Self::Node(node) => node.keys(),
            Self::Leaf(leaf) => leaf.keys(),
        }
    }

    pub fn keys_mut(&mut self) -> &mut [K] {
        match self {
            Self::Node(node) => node.keys_mut(),
            Self::Leaf(leaf) => leaf.keys_mut(),
        }
    }

    pub fn values(&self) -> &[V] {
        match self {
            Self::Node(node) => node.values(),
            Self::Leaf(leaf) => leaf.values(),
        }
    }

    pub fn values_mut(&mut self) -> &mut [V] {
        match self {
            Self::Node(node) => node.values_mut(),
            Self::Leaf(leaf) => leaf.values_mut(),
        }
    }

    #[inline]
    fn replace_with_child(&mut self, node_alloc: &mut SlabAllocator<Node<K, V>>) -> bool {
        assert_eq!(self.num_elements(), 0);

        match self {
            Self::Node(node) => unsafe {
                let mut child = node._children.pop(&mut 1).unwrap();
                // *child.parent_mut() = *node.parent_mut();
                mem::swap(self, &mut child);

                match child {
                    Self::Node(node) => node.free_forget(node_alloc),
                    Self::Leaf(_) => unreachable!(),
                };

                true
            },
            Self::Leaf(_) => false,
        }
    }
}

impl<'a, K: Ord, V> ChildRef<'a, K, V> {
    pub fn num_elements(&self) -> usize {
        match self {
            Self::Node(node) => node.num_elements(),
            Self::Leaf(leaf) => leaf.len(),
        }
    }

    // pub fn parent(&self) -> *const Node<K, V> {
    //     match self {
    //         Self::Node(node) => node.parent(),
    //         Self::Leaf(leaf) => leaf.parent,
    //     }
    // }

    pub fn keys(&self) -> &'a [K] {
        match self {
            Self::Node(node) => node.keys(),
            Self::Leaf(leaf) => leaf.keys(),
        }
    }

    pub fn values(&self) -> &'a [V] {
        match self {
            Self::Node(node) => node.values(),
            Self::Leaf(leaf) => leaf.values(),
        }
    }

    pub fn try_into_node(self) -> Option<&'a Node<K, V>> {
        match self {
            Self::Node(node) => Some(node),
            Self::Leaf(_) => None,
        }
    }

    pub fn try_into_leaf(self) -> Option<&'a NodeElements<K, V>> {
        match self {
            Self::Leaf(leaf) => Some(leaf),
            Self::Node(_) => None,
        }
    }
}

impl<'a, K: Ord, V> Clone for ChildRef<'a, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, K: Ord, V> Copy for ChildRef<'a, K, V> {}

impl<'a, K: Ord, V> ChildRefMut<'a, K, V> {
    pub fn num_elements(&self) -> usize {
        match self {
            Self::Node(node) => node.num_elements(),
            Self::Leaf(leaf) => leaf.len(),
        }
    }

    // pub fn parent(&self) -> *const Node<K, V> {
    //     match self {
    //         Self::Node(node) => node.parent(),
    //         Self::Leaf(leaf) => leaf.parent,
    //     }
    // }

    // pub fn parent_mut(&mut self) -> &mut *mut Node<K, V> {
    //     match self {
    //         Self::Node(node) => node.parent_mut(),
    //         Self::Leaf(leaf) => &mut leaf.parent,
    //     }
    // }

    pub fn as_ref(self) -> ChildRef<'a, K, V> {
        match self {
            Self::Node(node) => ChildRef::Node(node),
            Self::Leaf(leaf) => ChildRef::Leaf(leaf),
        }
    }

    pub fn borrow(&self) -> ChildRef<K, V> {
        match self {
            Self::Node(node) => ChildRef::Node(node),
            Self::Leaf(leaf) => ChildRef::Leaf(leaf),
        }
    }

    pub fn borrow_mut(&mut self) -> ChildRefMut<K, V> {
        match self {
            Self::Node(node) => ChildRefMut::Node(node),
            Self::Leaf(leaf) => ChildRefMut::Leaf(leaf),
        }
    }

    pub fn try_into_node(self) -> Option<&'a mut Node<K, V>> {
        match self {
            Self::Node(node) => Some(node),
            Self::Leaf(_) => None,
        }
    }

    pub fn try_into_leaf(self) -> Option<&'a mut NodeElements<K, V>> {
        match self {
            Self::Leaf(leaf) => Some(leaf),
            Self::Node(_) => None,
        }
    }

    pub fn keys(&self) -> &[K] {
        match self {
            Self::Node(node) => node.keys(),
            Self::Leaf(leaf) => leaf.keys(),
        }
    }

    pub fn values(&self) -> &[V] {
        match self {
            Self::Node(node) => node.values(),
            Self::Leaf(leaf) => leaf.values(),
        }
    }

    pub fn keys_mut(&mut self) -> &mut [K] {
        match self {
            Self::Node(node) => node.keys_mut(),
            Self::Leaf(leaf) => leaf.keys_mut(),
        }
    }

    pub fn values_mut(&mut self) -> &mut [V] {
        match self {
            Self::Node(node) => node.values_mut(),
            Self::Leaf(leaf) => leaf.values_mut(),
        }
    }

    pub fn swap(&mut self, other: ChildRefMut<K, V>) {
        match (self, other) {
            (Self::Node(self_node), ChildRefMut::Node(other_node)) => {
                mem::swap(*self_node, other_node)
            }
            (Self::Leaf(self_leaf), ChildRefMut::Leaf(other_leaf)) => {
                mem::swap(*self_leaf, other_leaf)
            }
            _ => panic!("called `ChildRefMut` where `self` and `other` are different (one is a leaf and the other is a node)"),
        }
    }
}

impl<'a, K: Ord, V> ChildrenSlice<'a, K, V> {
    pub fn get(&self, i: usize) -> Option<ChildRef<'a, K, V>> {
        match self {
            Self::Nodes(nodes) => Some(ChildRef::Node(nodes.get(i)?)),
            Self::Leafs(leafs) => Some(ChildRef::Leaf(leafs.get(i)?)),
        }
    }

    pub fn slice<B: ops::RangeBounds<usize>>(&self, bounds: B) -> Option<Self> {
        let bounds = (
            BoundClonedExt::cloned(&bounds.start_bound()),
            BoundClonedExt::cloned(&bounds.end_bound()),
        );

        match self {
            Self::Nodes(nodes) => Some(Self::Nodes(nodes.get(bounds)?)),
            Self::Leafs(leafs) => Some(Self::Leafs(leafs.get(bounds)?)),
        }
    }

    pub fn try_into_nodes(self) -> Option<&'a [SlabBox<Node<K, V>>]> {
        match self {
            Self::Nodes(nodes) => Some(nodes),
            Self::Leafs(_) => None,
        }
    }

    pub fn try_into_leafs(self) -> Option<&'a [SlabBox<NodeElements<K, V>>]> {
        match self {
            Self::Leafs(leafs) => Some(leafs),
            Self::Nodes(_) => None,
        }
    }

    #[inline]
    pub fn iter(&self) -> ChildrenIter<'a, K, V> {
        self.into_iter()
    }
}

impl<'a, K: Ord, V> Clone for ChildrenSlice<'a, K, V> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, K: Ord, V> Copy for ChildrenSlice<'a, K, V> {}

impl<'a, K: Ord, V> IntoIterator for ChildrenSlice<'a, K, V> {
    type Item = ChildRef<'a, K, V>;
    type IntoIter = ChildrenIter<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::Nodes(nodes) => ChildrenIter::Nodes(nodes.into_iter()),
            Self::Leafs(leafs) => ChildrenIter::Leafs(leafs.into_iter()),
        }
    }
}

impl<'a, K: Ord, V> ChildrenSliceMut<'a, K, V> {
    pub fn get(&self, i: usize) -> Option<ChildRef<K, V>> {
        match self {
            Self::Nodes(nodes) => Some(ChildRef::Node(nodes.get(i)?)),
            Self::Leafs(leafs) => Some(ChildRef::Leaf(leafs.get(i)?)),
        }
    }

    pub fn get_mut(&mut self, i: usize) -> Option<ChildRefMut<K, V>> {
        match self {
            Self::Nodes(nodes) => Some(ChildRefMut::Node(nodes.get_mut(i)?)),
            Self::Leafs(leafs) => Some(ChildRefMut::Leaf(leafs.get_mut(i)?)),
        }
    }

    pub fn drop_get(self, i: usize) -> Option<ChildRef<'a, K, V>> {
        match self {
            Self::Nodes(nodes) => Some(ChildRef::Node(nodes.get(i)?)),
            Self::Leafs(leafs) => Some(ChildRef::Leaf(leafs.get(i)?)),
        }
    }

    pub fn drop_get_mut(self, i: usize) -> Option<ChildRefMut<'a, K, V>> {
        match self {
            Self::Nodes(nodes) => Some(ChildRefMut::Node(nodes.get_mut(i)?)),
            Self::Leafs(leafs) => Some(ChildRefMut::Leaf(leafs.get_mut(i)?)),
        }
    }

    pub fn slice<B: ops::RangeBounds<usize>>(&self, bounds: B) -> Option<ChildrenSlice<K, V>> {
        let bounds = (
            BoundClonedExt::cloned(&bounds.start_bound()),
            BoundClonedExt::cloned(&bounds.end_bound()),
        );

        match self {
            Self::Nodes(nodes) => Some(ChildrenSlice::Nodes(nodes.get(bounds)?)),
            Self::Leafs(leafs) => Some(ChildrenSlice::Leafs(leafs.get(bounds)?)),
        }
    }

    pub fn slice_mut<B: ops::RangeBounds<usize>>(
        &mut self,
        bounds: B,
    ) -> Option<ChildrenSliceMut<K, V>> {
        let bounds = (
            BoundClonedExt::cloned(&bounds.start_bound()),
            BoundClonedExt::cloned(&bounds.end_bound()),
        );

        match self {
            Self::Nodes(nodes) => Some(ChildrenSliceMut::Nodes(nodes.get_mut(bounds)?)),
            Self::Leafs(leafs) => Some(ChildrenSliceMut::Leafs(leafs.get_mut(bounds)?)),
        }
    }

    pub fn drop_slice_mut<B: ops::RangeBounds<usize>>(self, bounds: B) -> Option<Self> {
        let bounds = (
            BoundClonedExt::cloned(&bounds.start_bound()),
            BoundClonedExt::cloned(&bounds.end_bound()),
        );

        match self {
            Self::Nodes(nodes) => Some(Self::Nodes(nodes.get_mut(bounds)?)),
            Self::Leafs(leafs) => Some(Self::Leafs(leafs.get_mut(bounds)?)),
        }
    }

    pub fn try_into_nodes(self) -> Option<&'a mut [SlabBox<Node<K, V>>]> {
        match self {
            Self::Nodes(nodes) => Some(nodes),
            Self::Leafs(_) => None,
        }
    }

    pub fn try_into_leafs(self) -> Option<&'a mut [SlabBox<NodeElements<K, V>>]> {
        match self {
            Self::Leafs(leafs) => Some(leafs),
            Self::Nodes(_) => None,
        }
    }

    pub fn iter(&self) -> ChildrenIter<K, V> {
        match self {
            Self::Nodes(nodes) => ChildrenIter::Nodes(nodes.iter()),
            Self::Leafs(leafs) => ChildrenIter::Leafs(leafs.iter()),
        }
    }

    pub fn iter_mut(&mut self) -> ChildrenIterMut<K, V> {
        match self {
            Self::Nodes(nodes) => ChildrenIterMut::Nodes(nodes.iter_mut()),
            Self::Leafs(leafs) => ChildrenIterMut::Leafs(leafs.iter_mut()),
        }
    }

    pub fn swap(&mut self, i: usize, j: usize) {
        match self {
            Self::Nodes(nodes) => {
                nodes.swap(i, j);
            }
            Self::Leafs(leafs) => {
                leafs.swap(i, j);
            }
        }
    }
}

impl<'a, K: Ord, V> IntoIterator for ChildrenSliceMut<'a, K, V> {
    type Item = ChildRefMut<'a, K, V>;
    type IntoIter = ChildrenIterMut<'a, K, V>;

    fn into_iter(self) -> Self::IntoIter {
        match self {
            Self::Nodes(nodes) => ChildrenIterMut::Nodes(nodes.into_iter()),
            Self::Leafs(leafs) => ChildrenIterMut::Leafs(leafs.into_iter()),
        }
    }
}

impl<'a, K: Ord, V> ExactSizeIterator for ChildrenIter<'a, K, V> {
    fn len(&self) -> usize {
        match self {
            Self::Nodes(nodes) => nodes.len(),
            Self::Leafs(leafs) => leafs.len(),
        }
    }
}

impl<'a, K: Ord, V> Iterator for ChildrenIter<'a, K, V> {
    type Item = ChildRef<'a, K, V>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes_iter) => Some(ChildRef::Node(nodes_iter.next()?)),
            Self::Leafs(leafs_iter) => Some(ChildRef::Leaf(leafs_iter.next()?)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Nodes(nodes) => nodes.size_hint(),
            Self::Leafs(leafs) => leafs.size_hint(),
        }
    }

    fn count(self) -> usize {
        match self {
            Self::Nodes(nodes) => nodes.count(),
            Self::Leafs(leafs) => leafs.count(),
        }
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes) => Some(ChildRef::Node(nodes.nth(n)?)),
            Self::Leafs(leafs) => Some(ChildRef::Leaf(leafs.nth(n)?)),
        }
    }

    #[inline]
    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }
}

impl<'a, K: Ord, V> DoubleEndedIterator for ChildrenIter<'a, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes_iter) => Some(ChildRef::Node(nodes_iter.next_back()?)),
            Self::Leafs(leafs_iter) => Some(ChildRef::Leaf(leafs_iter.next_back()?)),
        }
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes_iter) => Some(ChildRef::Node(nodes_iter.nth_back(n)?)),
            Self::Leafs(leafs_iter) => Some(ChildRef::Leaf(leafs_iter.nth_back(n)?)),
        }
    }
}

impl<'a, K: Ord, V> core::iter::FusedIterator for ChildrenIter<'a, K, V> {}

impl<'a, K: Ord, V> Clone for ChildrenIter<'a, K, V> {
    fn clone(&self) -> Self {
        match self {
            Self::Nodes(iter) => Self::Nodes(iter.clone()),
            Self::Leafs(iter) => Self::Leafs(iter.clone()),
        }
    }
}

impl<'a, K: Ord, V> ExactSizeIterator for ChildrenIterMut<'a, K, V> {
    fn len(&self) -> usize {
        match self {
            Self::Nodes(nodes) => nodes.len(),
            Self::Leafs(leafs) => leafs.len(),
        }
    }
}

impl<'a, K: Ord, V> Iterator for ChildrenIterMut<'a, K, V> {
    type Item = ChildRefMut<'a, K, V>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes_iter) => Some(ChildRefMut::Node(nodes_iter.next()?)),
            Self::Leafs(leafs_iter) => Some(ChildRefMut::Leaf(leafs_iter.next()?)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Nodes(nodes) => nodes.size_hint(),
            Self::Leafs(leafs) => leafs.size_hint(),
        }
    }

    fn count(self) -> usize {
        match self {
            Self::Nodes(nodes) => nodes.count(),
            Self::Leafs(leafs) => leafs.count(),
        }
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes) => Some(ChildRefMut::Node(nodes.nth(n)?)),
            Self::Leafs(leafs) => Some(ChildRefMut::Leaf(leafs.nth(n)?)),
        }
    }

    #[inline]
    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }
}

impl<'a, K: Ord, V> DoubleEndedIterator for ChildrenIterMut<'a, K, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes_iter) => Some(ChildRefMut::Node(nodes_iter.next_back()?)),
            Self::Leafs(leafs_iter) => Some(ChildRefMut::Leaf(leafs_iter.next_back()?)),
        }
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes_iter) => Some(ChildRefMut::Node(nodes_iter.nth_back(n)?)),
            Self::Leafs(leafs_iter) => Some(ChildRefMut::Leaf(leafs_iter.nth_back(n)?)),
        }
    }
}

impl<'a, K: Ord, V> core::iter::FusedIterator for ChildrenIterMut<'a, K, V> {}

impl<K: Ord, V> OuterLenChildren<K, V> {
    #[must_use]
    #[inline]
    pub fn new_nodes() -> Self {
        Self::Nodes(OuterLenStackVec::new())
    }

    #[must_use]
    #[inline]
    pub fn new_leafs() -> Self {
        Self::Leafs(OuterLenStackVec::new())
    }

    #[must_use]
    #[inline]
    pub unsafe fn push(&mut self, len: &mut usize, child: Child<K, V>) -> Option<Child<K, V>> {
        match (self, child) {
            (Self::Nodes(nodes), Child::Node(node)) => Some(Child::Node(nodes.push(len, node)?)),
            (Self::Leafs(leafs), Child::Leaf(leaf)) => Some(Child::Leaf(leafs.push(len, leaf)?)),
            _ => panic!("called `OuterLenChildren::push` where `self` and `child` are different (one is a leaf and the other is a node)"),
        }
    }

    #[must_use]
    #[inline]
    pub unsafe fn insert(
        &mut self,
        len: &mut usize,
        idx: usize,
        child: Child<K, V>,
    ) -> Option<Child<K, V>> {
        match (self, child) {
            (Self::Nodes(nodes), Child::Node(node)) => {
                Some(Child::Node(nodes.insert(len, idx, node)?))
            }
            (Self::Leafs(leafs), Child::Leaf(leaf)) => {
                Some(Child::Leaf(leafs.insert(len, idx, leaf)?))
            }
            _ => panic!("called `OuterLenChildren::insert` where `self` and `child` are different (one is a leaf and the other is a node)"),
        }
    }

    #[inline]
    pub unsafe fn pop(&mut self, len: &mut usize) -> Option<Child<K, V>> {
        match self {
            Self::Nodes(nodes) => Some(Child::Node(nodes.pop(len)?)),
            Self::Leafs(leafs) => Some(Child::Leaf(leafs.pop(len)?)),
        }
    }

    #[inline]
    pub unsafe fn remove(&mut self, len: &mut usize, idx: usize) -> Child<K, V> {
        match self {
            Self::Nodes(nodes) => Child::Node(nodes.remove(len, idx)),
            Self::Leafs(leafs) => Child::Leaf(leafs.remove(len, idx)),
        }
    }

    #[inline]
    pub unsafe fn split_at(&mut self, len: &mut usize, left_len: usize) -> Children<K, V> {
        match self {
            Self::Nodes(nodes) => {
                let (right_nodes, right_len) = nodes.split_at(len, left_len).into_raw_parts();
                Children::from_raw_parts(Self::Nodes(right_nodes), right_len)
            }
            Self::Leafs(leafs) => {
                let (right_leafs, right_len) = leafs.split_at(len, left_len).into_raw_parts();
                Children::from_raw_parts(Self::Leafs(right_leafs), right_len)
            }
        }
    }

    #[inline]
    pub unsafe fn as_slice(&self, len: usize) -> ChildrenSlice<K, V> {
        match self {
            Self::Nodes(nodes) => ChildrenSlice::Nodes(nodes.as_slice(len)),
            Self::Leafs(leafs) => ChildrenSlice::Leafs(leafs.as_slice(len)),
        }
    }

    #[inline]
    pub unsafe fn as_slice_mut(&mut self, len: usize) -> ChildrenSliceMut<K, V> {
        match self {
            Self::Nodes(nodes) => ChildrenSliceMut::Nodes(nodes.as_slice_mut(len)),
            Self::Leafs(leafs) => ChildrenSliceMut::Leafs(leafs.as_slice_mut(len)),
        }
    }

    // #[inline]
    // pub unsafe fn clone(&self, len: usize) -> Children<K, V>
    // where
    //     K: Clone,
    //     V: Clone,
    // {
    //     match self {
    //         Self::Nodes(nodes) => {
    //             let (nodes, len) = nodes.clone(len).into_raw_parts();
    //             Children::from_raw_parts(Self::Nodes(nodes), len)
    //         }
    //         Self::Leafs(leafs) => {
    //             let (leafs, len) = leafs.clone(len).into_raw_parts();
    //             Children::from_raw_parts(Self::Leafs(leafs), len)
    //         }
    //     }
    // }
}

impl<K: Ord, V> Children<K, V> {
    #[must_use]
    #[inline]
    pub fn new_nodes() -> Self {
        Self {
            _data: OuterLenChildren::Nodes(OuterLenStackVec::new()),
            _len: 0,
        }
    }

    #[must_use]
    #[inline]
    pub fn new_leafs() -> Self {
        Self {
            _data: OuterLenChildren::Leafs(OuterLenStackVec::new()),
            _len: 0,
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self._len
    }

    #[must_use]
    pub fn push(&mut self, child: Child<K, V>) -> Option<Child<K, V>> {
        match (&mut self._data, child) {
            (OuterLenChildren::Nodes(nodes), Child::Node(node)) => Some(Child::Node(unsafe { nodes.push(&mut self._len, node)? })),
            (OuterLenChildren::Leafs(leafs), Child::Leaf(leaf)) => Some(Child::Leaf(unsafe { leafs.push(&mut self._len, leaf)? })),
            _ => panic!("called `Children::push` where `self` and `child` are different (one is a leaf and the other is a node)"),
        }
    }

    #[must_use]
    pub fn insert(&mut self, idx: usize, child: Child<K, V>) -> Option<Child<K, V>> {
        match (&mut self._data, child) {
            (OuterLenChildren::Nodes(nodes), Child::Node(node)) => {
                Some(Child::Node(unsafe { nodes.insert(&mut self._len, idx, node)? }))
            }
            (OuterLenChildren::Leafs(leafs), Child::Leaf(leaf)) => {
                Some(Child::Leaf(unsafe { leafs.insert(&mut self._len, idx, leaf)? }))
            }
            _ => panic!("called `Children::insert` where `self` and `child` are different (one is a leaf and the other is a node)"),
        }
    }

    pub fn pop(&mut self) -> Option<Child<K, V>> {
        match &mut self._data {
            OuterLenChildren::Nodes(nodes) => {
                Some(Child::Node(unsafe { nodes.pop(&mut self._len)? }))
            }
            OuterLenChildren::Leafs(leafs) => {
                Some(Child::Leaf(unsafe { leafs.pop(&mut self._len)? }))
            }
        }
    }

    pub fn remove(&mut self, idx: usize) -> Child<K, V> {
        match &mut self._data {
            OuterLenChildren::Nodes(nodes) => {
                Child::Node(unsafe { nodes.remove(&mut self._len, idx) })
            }
            OuterLenChildren::Leafs(leafs) => {
                Child::Leaf(unsafe { leafs.remove(&mut self._len, idx) })
            }
        }
    }

    pub fn split_at(&mut self, left_len: usize) -> Self {
        match &mut self._data {
            OuterLenChildren::Nodes(nodes) => unsafe {
                let (right_data, right_len) =
                    nodes.split_at(&mut self._len, left_len).into_raw_parts();
                Self::from_raw_parts(OuterLenChildren::Nodes(right_data), right_len)
            },
            OuterLenChildren::Leafs(leafs) => unsafe {
                let (right_data, right_len) =
                    leafs.split_at(&mut self._len, left_len).into_raw_parts();
                Self::from_raw_parts(OuterLenChildren::Leafs(right_data), right_len)
            },
        }
    }

    #[inline]
    pub fn as_slice(&self) -> ChildrenSlice<K, V> {
        match &self._data {
            OuterLenChildren::Nodes(nodes) => {
                ChildrenSlice::Nodes(unsafe { nodes.as_slice(self.len()) })
            }
            OuterLenChildren::Leafs(leafs) => {
                ChildrenSlice::Leafs(unsafe { leafs.as_slice(self.len()) })
            }
        }
    }

    #[inline]
    pub fn as_slice_mut(&mut self) -> ChildrenSliceMut<K, V> {
        let len = self.len();
        match &mut self._data {
            OuterLenChildren::Nodes(nodes) => {
                ChildrenSliceMut::Nodes(unsafe { nodes.as_slice_mut(len) })
            }
            OuterLenChildren::Leafs(leafs) => {
                ChildrenSliceMut::Leafs(unsafe { leafs.as_slice_mut(len) })
            }
        }
    }

    #[inline]
    unsafe fn from_raw_parts(children: OuterLenChildren<K, V>, len: usize) -> Self {
        Self {
            _data: children,
            _len: len,
        }
    }

    #[inline]
    fn into_raw_parts(self) -> (OuterLenChildren<K, V>, usize) {
        unsafe {
            let mb = mem::ManuallyDrop::new(self);
            (ptr::read(&mb._data), mb._len)
        }
    }
}

impl<K: Ord, V> IntoIterator for Children<K, V> {
    type Item = Child<K, V>;
    type IntoIter = ChildrenIntoIter<K, V>;

    fn into_iter(self) -> ChildrenIntoIter<K, V> {
        unsafe {
            let (data, len) = self.into_raw_parts();
            match data {
                OuterLenChildren::Nodes(data) => {
                    ChildrenIntoIter::Nodes(StackVec::from_raw_parts(data, len).into_iter())
                }
                OuterLenChildren::Leafs(data) => {
                    ChildrenIntoIter::Leafs(StackVec::from_raw_parts(data, len).into_iter())
                }
            }
        }
    }
}

impl<K: Ord, V> Drop for Children<K, V> {
    fn drop(&mut self) {
        while let Some(_) = self.pop() {}
    }
}

// impl<K: Ord + Clone, V: Clone> Clone for Children<K, V> {
//     fn clone(&self) -> Self {
//         unsafe { self._data.clone(self._len) }
//     }
// }

impl<K: Ord + fmt::Debug, V: fmt::Debug> fmt::Debug for Children<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl<K: Ord, V> Into<ChildrenStackVec<K, V>> for Children<K, V> {
    #[inline]
    fn into(self) -> ChildrenStackVec<K, V> {
        unsafe {
            let (data, len) = self.into_raw_parts();

            match data {
                OuterLenChildren::Nodes(nodes) => {
                    ChildrenStackVec::Nodes(StackVec::from_raw_parts(nodes, len))
                }
                OuterLenChildren::Leafs(leafs) => {
                    ChildrenStackVec::Leafs(StackVec::from_raw_parts(leafs, len))
                }
            }
        }
    }
}

impl<K: Ord, V> Into<Children<K, V>> for ChildrenStackVec<K, V> {
    #[inline]
    fn into(self) -> Children<K, V> {
        unsafe {
            match self {
                ChildrenStackVec::Nodes(nodes) => {
                    let (data, len) = nodes.into_raw_parts();
                    Children::from_raw_parts(OuterLenChildren::Nodes(data), len)
                }
                ChildrenStackVec::Leafs(leafs) => {
                    let (data, len) = leafs.into_raw_parts();
                    Children::from_raw_parts(OuterLenChildren::Leafs(data), len)
                }
            }
        }
    }
}

impl<K: Ord, V> Iterator for ChildrenIntoIter<K, V> {
    type Item = Child<K, V>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes) => Some(Child::Node(nodes.next()?)),
            Self::Leafs(leafs) => Some(Child::Leaf(leafs.next()?)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Nodes(nodes) => nodes.size_hint(),
            Self::Leafs(leafs) => leafs.size_hint(),
        }
    }

    fn count(self) -> usize {
        match self {
            Self::Nodes(nodes) => nodes.count(),
            Self::Leafs(leafs) => leafs.count(),
        }
    }
}

impl<K: Ord, V> DoubleEndedIterator for ChildrenIntoIter<K, V> {
    #[inline]
    fn next_back(&mut self) -> Option<Self::Item> {
        match self {
            Self::Nodes(nodes) => Some(Child::Node(nodes.next_back()?)),
            Self::Leafs(leafs) => Some(Child::Leaf(leafs.next_back()?)),
        }
    }
}

impl<K: Ord, V> ExactSizeIterator for ChildrenIntoIter<K, V> {
    #[inline]
    fn len(&self) -> usize {
        match self {
            Self::Nodes(nodes) => nodes.len(),
            Self::Leafs(leafs) => leafs.len(),
        }
    }
}

#[repr(C)]
struct Node<K: Ord, V> {
    _elements: NodeElements<K, V>,
    _children: OuterLenChildren<K, V>,
}

impl<K: Ord, V> Node<K, V> {
    // pub fn new(child: Child<K, V>, parent: *mut Self) -> SlabBox<Self> {
    pub fn new(alloc: &mut SlabAllocator<Self>, child: Child<K, V>) -> SlabBox<Self> {
        unsafe {
            match child {
                Child::Node(node) => {
                    let mut slf = SlabBox::new(
                        alloc,
                        Self::from_raw_parts(
                            // NodeElements::new(parent),
                            NodeElements::new(),
                            OuterLenChildren::Nodes(OuterLenStackVec::new()),
                        ),
                    );
                    // *node.parent_mut() = slf.as_mut();
                    slf._children.push(&mut 0, Child::Node(node)).assert_none();
                    slf
                }
                Child::Leaf(leaf) => {
                    let mut slf = SlabBox::new(
                        alloc,
                        Self::from_raw_parts(
                            // NodeElements::new(parent),
                            NodeElements::new(),
                            OuterLenChildren::Leafs(OuterLenStackVec::new()),
                        ),
                    );
                    // leaf.parent = slf.as_mut();
                    slf._children.push(&mut 0, Child::Leaf(leaf)).assert_none();
                    slf
                }
            }
        }
    }

    #[inline]
    pub fn keys(&self) -> &[K] {
        self._elements.keys()
    }

    #[inline]
    pub fn keys_mut(&mut self) -> &mut [K] {
        self._elements.keys_mut()
    }

    #[inline]
    pub fn values(&self) -> &[V] {
        self._elements.values()
    }

    #[inline]
    pub fn values_mut(&mut self) -> &mut [V] {
        self._elements.values_mut()
    }

    #[inline]
    pub fn children(&self) -> ChildrenSlice<K, V> {
        unsafe { self._children.as_slice(self.num_children()) }
    }

    #[inline]
    pub fn children_mut(&mut self) -> ChildrenSliceMut<K, V> {
        unsafe { self._children.as_slice_mut(self.num_children()) }
    }

    #[inline]
    pub fn get_all_mut(&mut self) -> (&mut [K], &mut [V], ChildrenSliceMut<K, V>) {
        let children = unsafe { self._children.as_slice_mut(self.num_children()) };
        let (keys, values) = self._elements.get_all_mut();
        (keys, values, children)
    }

    #[must_use]
    pub fn push(&mut self, k: K, value: V, child: Child<K, V>) -> Option<(K, V, Child<K, V>)> {
        unsafe {
            let mut old_num_children = self.num_children();

            let overflow_element = self._elements.push(k, value);

            // *child.parent_mut() = self;
            let overflow_child = self._children.push(&mut old_num_children, child);

            match (overflow_element, overflow_child) {
                (Some((overflow_k, overflow_value)), Some(overflow_child)) => {
                    Some((overflow_k, overflow_value, overflow_child))
                }
                (None, None) => None,
                _ => unreachable!(),
            }
        }
    }

    #[must_use]
    pub fn insert(
        &mut self,
        idx: usize,
        k: K,
        value: V,
        child: Child<K, V>,
    ) -> Option<(K, V, Child<K, V>)> {
        unsafe {
            let mut old_num_children = self.num_children();

            let overflow_element = self._elements.insert(idx, k, value);

            // *child.parent_mut() = self;
            let overflow_child = self._children.insert(&mut old_num_children, idx + 1, child);

            match (overflow_element, overflow_child) {
                (Some((overflow_k, overflow_value)), Some(overflow_child)) => {
                    Some((overflow_k, overflow_value, overflow_child))
                }
                (None, None) => None,
                _ => unreachable!(),
            }
        }
    }

    pub fn pop(&mut self) -> Option<(K, V, Child<K, V>)> {
        unsafe {
            let mut old_num_children = self.num_children();

            let popped_element = self._elements.pop();

            if let Some((popped_k, popped_value)) = popped_element {
                let popped_child = self._children.pop(&mut old_num_children).unwrap();
                Some((popped_k, popped_value, popped_child))
            } else {
                None
            }
        }
    }

    pub fn remove(&mut self, idx: usize) -> (K, V, Child<K, V>) {
        unsafe {
            let mut old_num_children = self.num_children();

            let (removed_k, removed_values) = self._elements.remove(idx);
            let removed_child = self._children.remove(&mut old_num_children, idx + 1);

            (removed_k, removed_values, removed_child)
        }
    }

    // pub fn split(
    //     &mut self,
    //     rightmost_k: K,
    //     rightmost_value: V,
    //     rightmost_child: Child<K, V>,
    // ) -> (K, V, SlabBox<Self>) {
    //     unsafe {
    //         let mut num_children = self.num_children();

    //         let (sep_k, sep_value, right_elements) =
    //             self._elements.split(rightmost_k, rightmost_value);
    //         let mut right_children = self
    //             ._children
    //             .split_at(&mut num_children, self.num_children())
    //             .into_raw_parts()
    //             .0;
    //         right_children
    //             .push(&mut right_elements.len(), rightmost_child)
    //             .assert_none();

    //         //      [kv0| kv1 |kv2] kvr
    //         // [ch0, ch1||ch2, ch3] chr
    //         let right = SlabBox::new(Self::from_raw_parts(right_elements, right_children));

    //         // let right_mut_ptr = right.as_mut() as *mut Self;
    //         // match right.children_mut() {
    //         //     ChildrenSliceMut::Nodes(nodes) => {
    //         //         for node in nodes {
    //         //             *node.parent_mut() = right_mut_ptr;
    //         //         }
    //         //     }
    //         //     ChildrenSliceMut::Leafs(leafs) => {
    //         //         for leaf in leafs {
    //         //             leaf.parent = right_mut_ptr;
    //         //         }
    //         //     }
    //         // }

    //         (sep_k, sep_value, right)
    //     }
    // }

    pub fn split(
        &mut self,
        alloc: &mut SlabAllocator<Self>,
        rightmost_k: K,
        rightmost_value: V,
        rightmost_child: Child<K, V>,
    ) -> (K, V, SlabBox<Self>) {
        let mut right = SlabBox::new(
            alloc,
            Self {
                _elements: NodeElements::new(),
                _children: match self._children {
                    OuterLenChildren::Nodes(_) => OuterLenChildren::Nodes(OuterLenStackVec::new()),
                    OuterLenChildren::Leafs(_) => OuterLenChildren::Leafs(OuterLenStackVec::new()),
                },
            },
        );
        unsafe {
            let (sep_k, sep_value) =
                self._elements
                    .split(rightmost_k, rightmost_value, &mut right._elements);

            match (&self._children, &mut right._children, rightmost_child) {
                (
                    OuterLenChildren::Nodes(self_children),
                    OuterLenChildren::Nodes(right_children),
                    Child::Node(rightmost_child),
                ) => {
                    ptr::copy_nonoverlapping(
                        self_children.as_ptr().add(B + 1),
                        right_children.as_mut_ptr(),
                        B - 1,
                    );
                    right_children
                        .push(&mut (B - 1), rightmost_child)
                        .assert_none();
                }
                (
                    OuterLenChildren::Leafs(self_children),
                    OuterLenChildren::Leafs(right_children),
                    Child::Leaf(rightmost_child),
                ) => {
                    ptr::copy_nonoverlapping(
                        self_children.as_ptr().add(B + 1),
                        right_children.as_mut_ptr(),
                        B - 1,
                    );
                    right_children
                        .push(&mut (B - 1), rightmost_child)
                        .assert_none();
                }
                _ => unreachable!(),
            }

            (sep_k, sep_value, right)
        }
    }

    #[inline]
    pub fn merge(
        &mut self,
        alloc: &mut SlabAllocator<Self>,
        sep_k: K,
        sep_value: V,
        mut right: SlabBox<Self>,
    ) {
        assert!(self.num_elements() + right.num_elements() < MAX_NUM_ELEMENTS);
        unsafe {
            match (&mut self._children, &right._children) {
                (
                    OuterLenChildren::Nodes(self_children),
                    OuterLenChildren::Nodes(right_children),
                ) => {
                    ptr::copy_nonoverlapping(
                        right_children.as_ptr(),
                        self_children.as_mut_ptr().add(self.num_children()),
                        right.num_children(),
                    );
                }
                (
                    OuterLenChildren::Leafs(self_children),
                    OuterLenChildren::Leafs(right_children),
                ) => {
                    ptr::copy_nonoverlapping(
                        right_children.as_ptr(),
                        self_children.as_mut_ptr().add(self.num_children()),
                        right.num_children(),
                    );
                }
                _ => unreachable!(),
            }
            self._elements.merge(sep_k, sep_value, &mut right._elements);
            right.free_forget(alloc);
        }
    }

    fn into_raw_parts(self) -> (NodeElements<K, V>, OuterLenChildren<K, V>) {
        unsafe {
            let mb = mem::ManuallyDrop::new(self);
            (ptr::read(&mb._elements), ptr::read(&mb._children))
        }
    }

    unsafe fn from_raw_parts(
        elements: NodeElements<K, V>,
        children: OuterLenChildren<K, V>,
    ) -> Self {
        Self {
            _elements: elements,
            _children: children,
        }
    }

    pub fn separate(self) -> (NodeElements<K, V>, Children<K, V>) {
        unsafe {
            let num_children = self.num_children();

            let (elements, children) = self.into_raw_parts();

            (elements, Children::from_raw_parts(children, num_children))
        }
    }

    #[inline(always)]
    pub fn num_children(&self) -> usize {
        self.num_elements() + 1
    }
    #[inline(always)]
    pub fn num_elements(&self) -> usize {
        self._elements.len()
    }

    // #[inline(always)]
    // pub fn parent(&self) -> *const Self {
    //     self._elements.parent
    // }

    // #[inline(always)]
    // pub fn parent_mut(&mut self) -> &mut *mut Self {
    //     &mut self._elements.parent
    // }

    #[inline(always)]
    pub unsafe fn set_num_elements(&mut self, num_elements: usize) {
        self._elements.set_len(num_elements);
    }
}

// impl<K: Ord + Clone, V: Clone> Clone for Node<K, V> {
//     fn clone(&self) -> Self {
//         unsafe {
//             Self::from_raw_parts(
//                 self._elements.clone(),
//                 self._children.clone(self.num_children()).into_raw_parts().0,
//             )
//         }
//     }
// }

impl<K: Ord + fmt::Debug, V: fmt::Debug> fmt::Debug for Node<K, V> {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            // .field("elements", &self._elements)
            .field("keys", &self.keys())
            .field("values", &self.values())
            .field("children", &self.children())
            .finish()
    }
}

impl<K: Ord, V> Drop for Node<K, V> {
    fn drop(&mut self) {
        while let Some(_) = self.pop() {}
        unsafe {
            self._children.pop(&mut 1).unwrap();
        }
    }
}

// #[inline]
// pub fn lin_search<T, Q>(a: &[T], k: &Q) -> Result<usize, usize>
// where
//     T: Ord + Borrow<Q>,
//     Q: Ord + ?Sized,
// {
//     lin_search_by(a, |x| x.borrow().cmp(k))
// }
//
// #[inline]
// pub fn lin_search_by<T, F: FnMut(&T) -> Ordering>(a: &[T], mut f: F) -> Result<usize, usize> {
//     // const LIN_SEARCH_SIZE: usize = 20;
//
//     // let mut size = a.len();
//     // let mut left = 0;
//     // let mut right = size;
//
//     // while LIN_SEARCH_SIZE < size {
//     //     let mid = left + size / 2;
//
//     //     let cmp = f(unsafe { a.get_unchecked(mid) });
//
//     //     if cmp == Ordering::Less {
//     //         left = mid + 1;
//     //     } else if cmp == Ordering::Greater {
//     //         right = mid;
//     //     } else {
//     //         return Ok(mid);
//     //     }
//     //     size = right - left;
//     // }
//
//     for (i, x) in a.iter().enumerate() {
//         let cmp = f(x);
//         if cmp == Ordering::Greater {
//             return Err(i);
//         } else if cmp == Ordering::Equal {
//             return Ok(i);
//         }
//     }
//     Err(a.len())
// }

// #[derive(Debug, Clone)]
#[derive(Debug)]
pub struct BTree<K: Ord, V> {
    root: Child<K, V>,
    len: usize,
    depth: usize,

    node_alloc: SlabAllocator<Node<K, V>>,
    leaf_alloc: SlabAllocator<NodeElements<K, V>>,
}

impl<K: Ord, V> BTree<K, V> {
    pub fn new(chunk: &'static mut [u8]) -> Self {
        // N := total num of the nodes and leafs
        // NN := num nodes
        // NL := num leafs
        //
        // NN = N * 1 / B
        // NL = N * (B - 1) / B
        //
        // S := total size of the nodes and leafs
        // S1N := size of a single node
        // S1L := size of a single leaf
        // SN := total size of the nodes
        // SL := total size of the leafs
        // SPN := percentage of nodes in the allocated space
        // SPL := percentage of leafs in the allocated space
        //
        // SN = NN * S1N = N * S1N * 1 / B
        // SL = NL * S1L = N * S1L * (B - 1) / B
        //
        // SPN = SN / (SN + SL) = S1N / (S1N + S1L * (B - 1))
        // SPL = SL / (SN + SL) = S1L * (B - 1) / (S1N + S1L * (B - 1))

        let (node_alloc_chunk, leaf_alloc_chunk) = chunk.split_at_mut(
            chunk.len() * mem::size_of::<Node<K, V>>()
                / (mem::size_of::<Node<K, V>>() + (B - 1) * mem::size_of::<NodeElements<K, V>>()),
        );

        let node_alloc = SlabAllocator::new(node_alloc_chunk);
        let mut leaf_alloc = SlabAllocator::new(leaf_alloc_chunk);

        Self {
            // root: Child::Leaf(SlabBox::new(NodeElements::new(ptr::null_mut()))),
            root: Child::Leaf(SlabBox::new(&mut leaf_alloc, NodeElements::new())),
            len: 0,
            depth: 1,

            leaf_alloc,
            node_alloc,
        }
    }

    #[inline]
    pub fn needs_new_chunk(&self) -> bool {
        self.node_alloc.needs_new_chunk() || self.leaf_alloc.needs_new_chunk()
    }

    pub fn add_chunk(&mut self, chunk: &'static mut [u8]) {
        if self.node_alloc.needs_new_chunk() {
            self.node_alloc.add_chunk(chunk);
        } else if self.leaf_alloc.needs_new_chunk() {
            self.leaf_alloc.add_chunk(chunk);
        } else {
            let (node_alloc_chunk, leaf_alloc_chunk) = chunk.split_at_mut(
                chunk.len() * mem::size_of::<Node<K, V>>()
                    / (mem::size_of::<Node<K, V>>()
                        + (B - 1) * mem::size_of::<NodeElements<K, V>>()),
            );

            self.node_alloc.add_chunk(node_alloc_chunk);
            self.leaf_alloc.add_chunk(leaf_alloc_chunk);
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let mut child = self.root.as_ref();
        'get_loop: loop {
            match child {
                ChildRef::Node(node) => {
                    for (i, elem_k) in node.keys().iter().enumerate() {
                        let ord = key.cmp(elem_k.borrow());
                        match ord {
                            Ordering::Less => {
                                child = node.children().get(i).unwrap();
                                continue 'get_loop;
                            }
                            Ordering::Equal => {
                                return Some(&node.values()[i]);
                            }
                            Ordering::Greater => {}
                        }
                    }
                    let last_child_idx = node.num_elements();
                    child = node.children().get(last_child_idx).unwrap();
                }
                ChildRef::Leaf(leaf) => {
                    for (i, elem_k) in leaf.keys().iter().enumerate() {
                        let ord = key.cmp(elem_k.borrow());
                        match ord {
                            Ordering::Less => return None,
                            Ordering::Equal => {
                                return Some(&leaf.values()[i]);
                            }
                            Ordering::Greater => {}
                        }
                    }
                    return None;
                }
            }
        }
    }

    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let mut child = self.root.as_mut();
        'get_loop: loop {
            match child {
                ChildRefMut::Node(node) => {
                    for (i, elem_k) in node.keys().iter().enumerate() {
                        match key.cmp(elem_k.borrow()) {
                            Ordering::Less => {
                                child = node.children_mut().drop_get_mut(i).unwrap();
                                continue 'get_loop;
                            }
                            Ordering::Equal => {
                                return Some(&mut node.values_mut()[i]);
                            }
                            Ordering::Greater => {}
                        }
                    }
                    let last_child_idx = node.num_elements();
                    child = node.children_mut().drop_get_mut(last_child_idx).unwrap();
                }
                ChildRefMut::Leaf(leaf) => {
                    for (i, elem_k) in leaf.keys().iter().enumerate() {
                        match key.cmp(elem_k.borrow()) {
                            Ordering::Less => return None,
                            Ordering::Equal => {
                                return Some(&mut leaf.values_mut()[i]);
                            }
                            Ordering::Greater => {}
                        }
                    }
                    return None;
                }
            }
        }
    }

    pub fn insert(&mut self, key: K, value: V) -> Option<(K, V)> {
        self.len += 1;
        match &mut self.root {
            Child::Leaf(root) => {
                let (overflow_k, overflow_value) = 'root_search_and_insert: loop {
                    for (i, elem_k) in root.keys().iter().enumerate() {
                        match key.cmp(elem_k) {
                            Ordering::Less => {
                                break 'root_search_and_insert root.insert(i, key, value)?;
                            }
                            Ordering::Equal => {
                                self.len -= 1;
                                return Some((key, mem::replace(&mut root.values_mut()[i], value)));
                            }
                            Ordering::Greater => {}
                        }
                    }

                    break 'root_search_and_insert root.push(key, value)?;
                };

                self.depth += 1;

                let mut right = SlabBox::new(&mut self.leaf_alloc, NodeElements::new());
                let (sep_k, sep_value) = root.split(overflow_k, overflow_value, right.as_mut());

                let left = mem::replace(root, right);
                let new_root = Node::new(&mut self.node_alloc, Child::Leaf(left));
                let right = mem::replace(&mut self.root, Child::Node(new_root))
                    .try_into_leaf()
                    .unwrap();

                self.root
                    .try_as_node_mut()
                    .unwrap()
                    .push(sep_k, sep_value, Child::Leaf(right))
                    .assert_none();

                return None;
            }
            Child::Node(root) => {
                let mut ref_stack = OnStackRefMutStack::<Node<K, V>, 20>::new();
                let mut children_indices_stack = StackVec::<usize, 20>::new(); // ;
                ref_stack.push_root(root.as_mut());

                let (mut overflow_k, mut overflow_value) = 'search_and_insert: loop {
                    let node = ref_stack.peek_mut().unwrap();

                    let mut child_idx = node.num_elements();
                    for (i, elem_k) in node.keys().iter().enumerate() {
                        match key.cmp(elem_k) {
                            Ordering::Less => {
                                child_idx = i;
                                break;
                            }
                            Ordering::Equal => {
                                self.len -= 1;
                                return Some((key, mem::replace(&mut node.values_mut()[i], value)));
                            }
                            Ordering::Greater => {}
                        }
                    }

                    children_indices_stack.push(child_idx).assert_none();
                    match node.children_mut() {
                        ChildrenSliceMut::Nodes(_) => assert!(ref_stack.push(|node| unsafe {
                            node.children_mut().try_into_nodes().unwrap_unchecked()[child_idx]
                                .as_mut()
                        })),
                        ChildrenSliceMut::Leafs(leafs) => {
                            let leaf = leafs[child_idx].as_mut();

                            for (i, elem_k) in leaf.keys().iter().enumerate() {
                                match key.cmp(elem_k) {
                                    Ordering::Less => {
                                        break 'search_and_insert leaf.insert(i, key, value)?;
                                    }
                                    Ordering::Equal => {
                                        self.len -= 1;
                                        return Some((
                                            key,
                                            mem::replace(&mut leaf.values_mut()[i], value),
                                        ));
                                    }
                                    Ordering::Greater => {}
                                }
                            }

                            break 'search_and_insert leaf.push(key, value)?;
                        }
                    }
                    // match ref_stack.try_push(|node| match node.children_mut() {
                    //     ChildrenSliceMut::Nodes(children_nodes) => {
                    //         Ok(children_nodes[child_idx].as_mut())
                    //     }
                    //     ChildrenSliceMut::Leafs(leafs) => Err(leafs[child_idx].as_mut()),
                    // }) {
                    //     Ok(check) => assert!(check),
                    //     Err(leaf) => {
                    //         for (i, elem_k) in leaf.keys().iter().enumerate() {
                    //             match key.cmp(elem_k) {
                    //                 Ordering::Less => {
                    //                     break 'search_and_insert leaf.insert(i, key, value)?;
                    //                 }
                    //                 Ordering::Equal => {
                    //                     self.len -= 1;
                    //                     return Some((
                    //                         key,
                    //                         mem::replace(&mut leaf.values_mut()[i], value),
                    //                     ));
                    //                 }
                    //                 Ordering::Greater => {}
                    //             }
                    //         }

                    //         break 'search_and_insert leaf.push(key, value)?;
                    //     }
                    // }
                };
                let mut overflow_child;

                // Leaf Overflow
                {
                    let node = ref_stack.peek_mut().unwrap();

                    let child_idx = children_indices_stack.pop().unwrap();
                    let child = node.children_mut().try_into_leafs().unwrap()[child_idx].as_mut();

                    let mut right = SlabBox::new(&mut self.leaf_alloc, NodeElements::new());
                    let (sep_k, sep_value) =
                        child.split(overflow_k, overflow_value, right.as_mut());

                    let (rightmost_k, rightmost_value, rightmost_child) =
                        node.insert(child_idx, sep_k, sep_value, Child::Leaf(right))?;

                    overflow_k = rightmost_k;
                    overflow_value = rightmost_value;
                    overflow_child = rightmost_child;
                }

                loop {
                    match ref_stack.pop() {
                        Some(_root) => {
                            drop(ref_stack);

                            self.depth += 1;

                            let (sep_k, sep_value, right) = root.split(
                                &mut self.node_alloc,
                                overflow_k,
                                overflow_value,
                                overflow_child,
                            );

                            let left = mem::replace(root, right);

                            let new_root = Node::new(&mut self.node_alloc, Child::Node(left));
                            let right = mem::replace(root, new_root);

                            root.push(sep_k, sep_value, Child::Node(right))
                                .assert_none();

                            return None;
                        }
                        None => {
                            let node = ref_stack.peek_mut().unwrap();

                            let child_idx = children_indices_stack.pop().unwrap();
                            let child =
                                node.children_mut().try_into_nodes().unwrap()[child_idx].as_mut();

                            let (sep_k, sep_value, right) = child.split(
                                &mut self.node_alloc,
                                overflow_k,
                                overflow_value,
                                overflow_child,
                            );

                            let (rightmost_k, rightmost_value, rightmost_child) =
                                node.insert(child_idx, sep_k, sep_value, Child::Node(right))?;

                            overflow_k = rightmost_k;
                            overflow_value = rightmost_value;
                            overflow_child = rightmost_child;
                        }
                    }
                }
            }
        }
    }

    pub fn remove<Q>(&mut self, key: &Q) -> Option<(K, V)>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        fn resolve_underflow<K: Ord, V>(
            leaf_alloc: &mut SlabAllocator<NodeElements<K, V>>,
            node_alloc: &mut SlabAllocator<Node<K, V>>,
            node: &mut Node<K, V>,
            child_idx: usize,
        ) {
            debug_assert!(
                node.children().get(child_idx).unwrap().num_elements() < MIN_NUM_ELEMENTS
            );

            let (keys, values, children) = node.get_all_mut();
            match children {
                ChildrenSliceMut::Nodes(children) => {
                    if let Some(donor_child) = children
                        .get_mut(child_idx + 1)
                        .filter(|child| MIN_NUM_ELEMENTS < child.num_elements())
                    {
                        donor_child.children_mut().swap(0, 1);
                        let (mut removed_k, mut removed_value, removed_child) =
                            donor_child.remove(0);

                        mem::swap(&mut keys[child_idx], &mut removed_k);
                        mem::swap(&mut values[child_idx], &mut removed_value);

                        children[child_idx]
                            .push(removed_k, removed_value, removed_child)
                            .assert_none();
                    } else if let Some(donor_child) = child_idx
                        .checked_sub(1)
                        .map(|i| &mut children[i])
                        .filter(|child| MIN_NUM_ELEMENTS < child.num_elements())
                    {
                        let (mut removed_k, mut removed_value, removed_child) =
                            donor_child.pop().unwrap();

                        mem::swap(&mut keys[child_idx - 1], &mut removed_k);
                        mem::swap(&mut values[child_idx - 1], &mut removed_value);

                        children[child_idx]
                            .insert(0, removed_k, removed_value, removed_child)
                            .assert_none();
                        children[child_idx].children_mut().swap(0, 1);
                    } else {
                        let left = child_idx.saturating_sub(1);

                        let (sep_k, sep_value, right_child) = node.remove(left);

                        let left_child = &mut node.children_mut().try_into_nodes().unwrap()[left];

                        let right_child = right_child.try_into_node().unwrap();

                        left_child.merge(node_alloc, sep_k, sep_value, right_child);
                    }
                }
                ChildrenSliceMut::Leafs(children) => {
                    if let Some(donor_child) = children
                        .get_mut(child_idx + 1)
                        .filter(|child| MIN_NUM_ELEMENTS < child.len())
                    {
                        let (mut removed_k, mut removed_value) = donor_child.remove(0);

                        mem::swap(&mut keys[child_idx], &mut removed_k);
                        mem::swap(&mut values[child_idx], &mut removed_value);

                        children[child_idx]
                            .push(removed_k, removed_value)
                            .assert_none();
                    } else if let Some(donor_child) = child_idx
                        .checked_sub(1)
                        .map(|i| &mut children[i])
                        .filter(|child| MIN_NUM_ELEMENTS < child.len())
                    {
                        let (mut removed_k, mut removed_value) = donor_child.pop().unwrap();

                        mem::swap(&mut keys[child_idx - 1], &mut removed_k);
                        mem::swap(&mut values[child_idx - 1], &mut removed_value);

                        children[child_idx]
                            .insert(0, removed_k, removed_value)
                            .assert_none();
                    } else {
                        let left = child_idx.saturating_sub(1);

                        let (sep_k, sep_value, right_child) = node.remove(left);

                        let left_child = &mut node.children_mut().try_into_leafs().unwrap()[left];

                        let mut right_child = right_child.try_into_leaf().unwrap();

                        left_child.merge(sep_k, sep_value, right_child.as_mut());
                        right_child.free(leaf_alloc);
                    }
                }
            }
            debug_assert!(node
                .children()
                .iter()
                .all(|child| MIN_NUM_ELEMENTS <= child.num_elements()));
        }

        match &mut self.root {
            Child::Leaf(root) => {
                for (i, elem_k) in root.keys().iter().enumerate() {
                    match key.cmp(elem_k.borrow()) {
                        Ordering::Less => {
                            return None;
                        }
                        Ordering::Equal => {
                            self.len -= 1;
                            return Some(root.remove(i));
                        }
                        Ordering::Greater => {}
                    }
                }

                None
            }
            Child::Node(root) => {
                let mut ref_stack = OnStackRefMutStack::<_, 20>::new();
                let mut children_indices_stack = StackVec::<_, 20>::new(); // ;
                ref_stack.push_root(root.as_mut());

                let (removed_k, removed_value) = 'search_and_remove: loop {
                    let node = ref_stack.peek_mut().unwrap();

                    let mut child_idx = node.num_elements();
                    for (i, elem_k) in node.keys().iter().enumerate() {
                        match key.cmp(elem_k.borrow()) {
                            Ordering::Less => {
                                child_idx = i;
                                break;
                            }
                            Ordering::Equal => {
                                children_indices_stack.push(i).assert_none();

                                let (replacement_k, replacement_value) = match node.children_mut() {
                                    ChildrenSliceMut::Nodes(_) => {
                                        assert!(ref_stack.push(|node| unsafe {
                                            node.children_mut().try_into_nodes().unwrap_unchecked()
                                                [i]
                                                .as_mut()
                                        }));
                                        loop {
                                            match ref_stack.peek_mut().unwrap().children_mut() {
                                                ChildrenSliceMut::Nodes(_) => assert!(ref_stack
                                                    .push(|node| unsafe {
                                                        node.children_mut()
                                                            .try_into_nodes()
                                                            .unwrap_unchecked()
                                                            .last_mut()
                                                            .unwrap_unchecked()
                                                            .as_mut()
                                                    })),
                                                ChildrenSliceMut::Leafs(leafs) => {
                                                    break leafs.last_mut().unwrap().pop().unwrap();
                                                }
                                            }
                                        }
                                    }
                                    ChildrenSliceMut::Leafs(leafs) => leafs[i].pop().unwrap(),
                                };
                                // match ref_stack.try_push(|node| match node.children_mut() {
                                //     ChildrenSliceMut::Nodes(nodes) => Ok(nodes[i].as_mut()),
                                //     ChildrenSliceMut::Leafs(leafs) => Err(leafs[i].as_mut()),
                                // }) {
                                //     Ok(check) => {
                                //         assert!(check);
                                //         loop {
                                //             match ref_stack.try_push(|node| {
                                //                 match node.children_mut() {
                                //                     ChildrenSliceMut::Nodes(nodes) => {
                                //                         Ok(nodes.last_mut().unwrap().as_mut())
                                //                     }
                                //                     ChildrenSliceMut::Leafs(leafs) => {
                                //                         Err(leafs.last_mut().unwrap().as_mut())
                                //                     }
                                //                 }
                                //             }) {
                                //                 Ok(success) => assert!(success),
                                //                 Err(leaf) => {
                                //                     break leaf.pop().unwrap();
                                //                 }
                                //             }
                                //         }
                                //     }
                                //     Err(leaf) => leaf.pop().unwrap(),
                                // };

                                while children_indices_stack.len() < ref_stack.len() {
                                    let node = ref_stack.peek_mut().unwrap();

                                    if node
                                        .children()
                                        .get(node.num_elements())
                                        .unwrap()
                                        .num_elements()
                                        < MIN_NUM_ELEMENTS
                                    {
                                        resolve_underflow(
                                            &mut self.leaf_alloc,
                                            &mut self.node_alloc,
                                            node,
                                            node.num_elements(),
                                        );
                                        ref_stack.pop().assert_none();
                                    } else {
                                        while children_indices_stack.len() < ref_stack.len() {
                                            ref_stack.pop().assert_none();
                                        }

                                        let node = ref_stack.peek_mut().unwrap();
                                        let removed_k =
                                            mem::replace(&mut node.keys_mut()[i], replacement_k);
                                        let removed_value = mem::replace(
                                            &mut node.values_mut()[i],
                                            replacement_value,
                                        );

                                        self.len -= 1;
                                        return Some((removed_k, removed_value));
                                    }
                                }

                                let node = ref_stack.peek_mut().unwrap();
                                let removed_k =
                                    mem::replace(&mut node.keys_mut()[i], replacement_k);
                                let removed_value =
                                    mem::replace(&mut node.values_mut()[i], replacement_value);

                                break 'search_and_remove (removed_k, removed_value);
                            }
                            Ordering::Greater => {}
                        }
                    }

                    children_indices_stack.push(child_idx).assert_none();
                    match node.children_mut() {
                        ChildrenSliceMut::Nodes(_) => assert!(ref_stack.push(|node| unsafe {
                            node.children_mut().try_into_nodes().unwrap_unchecked()[child_idx]
                                .as_mut()
                        })),
                        ChildrenSliceMut::Leafs(leafs) => {
                            let leaf = leafs[child_idx].as_mut();

                            for (i, elem_k) in leaf.keys().iter().enumerate() {
                                match key.cmp(elem_k.borrow()) {
                                    Ordering::Less => {
                                        return None;
                                    }
                                    Ordering::Equal => {
                                        if leaf.len() == MIN_NUM_ELEMENTS {
                                            break 'search_and_remove leaf.remove(i);
                                        } else {
                                            self.len -= 1;
                                            return Some(leaf.remove(i));
                                        }
                                    }
                                    Ordering::Greater => {}
                                }
                            }
                            return None;
                        }
                    }
                    // match ref_stack.try_push(|node| match node.children_mut() {
                    //     ChildrenSliceMut::Nodes(nodes) => Ok(nodes[child_idx].as_mut()),
                    //     ChildrenSliceMut::Leafs(leafs) => Err(leafs[child_idx].as_mut()),
                    // }) {
                    //     Ok(success) => assert!(success),
                    //     Err(leaf) => {
                    //         for (i, elem_k) in leaf.keys().iter().enumerate() {
                    //             match key.cmp(elem_k.borrow()) {
                    //                 Ordering::Less => {
                    //                     return None;
                    //                 }
                    //                 Ordering::Equal => {
                    //                     if leaf.len() == MIN_NUM_ELEMENTS {
                    //                         break 'search_and_remove leaf.remove(i);
                    //                     } else {
                    //                         self.len -= 1;
                    //                         return Some(leaf.remove(i));
                    //                     }
                    //                 }
                    //                 Ordering::Greater => {}
                    //             }
                    //         }
                    //         return None;
                    //     }
                    // }
                };

                loop {
                    let node = ref_stack.peek_mut().unwrap();
                    let child_idx = children_indices_stack.pop().unwrap();

                    if node.children().get(child_idx).unwrap().num_elements() < MIN_NUM_ELEMENTS {
                        resolve_underflow(
                            &mut self.leaf_alloc,
                            &mut self.node_alloc,
                            node,
                            child_idx,
                        );
                    } else {
                        self.len -= 1;
                        return Some((removed_k, removed_value));
                    }

                    if let Some(root) = ref_stack.pop() {
                        drop(ref_stack);

                        if root.num_elements() == 0 {
                            self.depth -= 1;
                            assert!(self.root.replace_with_child(&mut self.node_alloc));
                        }
                        self.len -= 1;
                        return Some((removed_k, removed_value));
                    }
                }
            }
        }
    }

    // pub fn iter(&self) -> BTreeIter<K, V> {
    //     let mut left = Vec::with_capacity(self.depth());
    //     left.push((self.root.as_ref(), 0));
    //     while let Some(&(ChildRef::Node(node), _)) = left.last() {
    //         left.push((node.children().get(0).unwrap(), 0));
    //     }

    //     let mut right = Vec::with_capacity(self.depth());
    //     right.push((self.root.as_ref(), self.root.num_elements()));
    //     while let Some(&(ChildRef::Node(node), child_idx)) = right.last() {
    //         let child = node.children().get(child_idx).unwrap();
    //         right.push((child, child.num_elements()));
    //     }

    //     BTreeIter {
    //         left,
    //         right,
    //         len: self.len(),
    //     }
    // }

    // pub fn iter_mut(&mut self) -> BTreeIterMut<K, V> {
    //     unsafe {
    //         let mut left = Vec::with_capacity(self.depth());
    //         left.push((
    //             match self.root.as_mut() {
    //                 ChildRefMut::Node(root) => ChildPtrMut::Node(root),
    //                 ChildRefMut::Leaf(root) => ChildPtrMut::Leaf(root),
    //             },
    //             0,
    //         ));

    //         while let Some(&(ChildPtrMut::Node(node), _)) = left.last() {
    //             left.push((
    //                 match (*node).children_mut() {
    //                     ChildrenSliceMut::Nodes(children) => ChildPtrMut::Node(&mut *children[0]),
    //                     ChildrenSliceMut::Leafs(children) => ChildPtrMut::Leaf(&mut *children[0]),
    //                 },
    //                 0,
    //             ));
    //         }

    //         let mut right = Vec::with_capacity(self.depth());
    //         right.push((
    //             match self.root.as_mut() {
    //                 ChildRefMut::Node(root) => ChildPtrMut::Node(root),
    //                 ChildRefMut::Leaf(root) => ChildPtrMut::Leaf(root),
    //             },
    //             self.root.num_elements(),
    //         ));
    //         while let Some(&(ChildPtrMut::Node(node), child_idx)) = right.last() {
    //             right.push(match (*node).children_mut() {
    //                 ChildrenSliceMut::Nodes(children) => {
    //                     let child = &mut *children[child_idx];
    //                     (ChildPtrMut::Node(child), child.num_elements())
    //                 }
    //                 ChildrenSliceMut::Leafs(children) => {
    //                     let child = &mut *children[child_idx];
    //                     (ChildPtrMut::Leaf(child), child.len())
    //                 }
    //             });
    //         }

    //         BTreeIterMut {
    //             left,
    //             right,
    //             len: self.len(),
    //             phantom: PhantomData,
    //         }
    //     }
    // }
}

// impl<K: Ord, V> Default for BTree<K, V> {
//     fn default() -> Self {
//         Self::new()
//     }
// }

// #[derive(Clone, Debug)]
// pub struct BTreeIter<'a, K: Ord, V> {
//     left: Vec<(ChildRef<'a, K, V>, usize)>,
//     right: Vec<(ChildRef<'a, K, V>, usize)>,
//     len: usize,
// }
//
// impl<'a, K: Ord, V> Iterator for BTreeIter<'a, K, V> {
//     type Item = (&'a K, &'a V);
//
//     fn next(&mut self) -> Option<Self::Item> {
//         if 0 < self.len {
//             self.len -= 1;
//             let (child, elem_idx) = self.left.last_mut().unwrap();
//             let item = (&child.keys()[*elem_idx], &child.values()[*elem_idx]);
//
//             *elem_idx += 1;
//             match *child {
//                 ChildRef::Node(_) => {
//                     while let Some(&(ChildRef::Node(node), child_idx)) = self.left.last() {
//                         self.left.push((node.children().get(child_idx).unwrap(), 0));
//                     }
//                 }
//                 ChildRef::Leaf(_) => {
//                     while let Some(&(child, elem_idx)) = self.left.last() {
//                         if child.num_elements() <= elem_idx {
//                             self.left.pop();
//                         }
//                     }
//                 }
//             }
//
//             Some(item)
//         } else {
//             None
//         }
//     }
//
//     fn size_hint(&self) -> (usize, Option<usize>) {
//         (self.len, Some(self.len))
//     }
//
//     fn count(self) -> usize {
//         self.len
//     }
// }
//
// impl<'a, K: Ord, V> DoubleEndedIterator for BTreeIter<'a, K, V> {
//     fn next_back(&mut self) -> Option<Self::Item> {
//         if 0 < self.len {
//             self.len -= 1;
//             let (child, elem_idx) = self.right.last_mut().unwrap();
//             *elem_idx -= 1;
//             let item = (&child.keys()[*elem_idx], &child.values()[*elem_idx]);
//
//             match *child {
//                 ChildRef::Node(_) => {
//                     while let Some(&(ChildRef::Node(node), child_idx)) = self.right.last() {
//                         let child = node.children().get(child_idx).unwrap();
//                         self.right.push((child, child.num_elements()));
//                     }
//                 }
//                 ChildRef::Leaf(_) => {
//                     while let Some(&(_, elem_idx)) = self.right.last() {
//                         if elem_idx == 0 {
//                             self.right.pop();
//                         }
//                     }
//                 }
//             }
//
//             Some(item)
//         } else {
//             None
//         }
//     }
// }
//
// impl<'a, K: Ord, V> ExactSizeIterator for BTreeIter<'a, K, V> {
//     fn len(&self) -> usize {
//         self.len
//     }
// }
//
// #[derive(Debug)]
// pub struct BTreeIterMut<'a, K: Ord, V> {
//     left: Vec<(ChildPtrMut<K, V>, usize)>,
//     right: Vec<(ChildPtrMut<K, V>, usize)>,
//     len: usize,
//     phantom: PhantomData<ChildRefMut<'a, K, V>>,
// }
//
// impl<'a, K: Ord, V> Iterator for BTreeIterMut<'a, K, V> {
//     type Item = (&'a K, &'a mut V);
//
//     fn next(&mut self) -> Option<Self::Item> {
//         if 0 < self.len {
//             self.len -= 1;
//             let (child, elem_idx) = self.left.last_mut().unwrap();
//
//             *elem_idx += 1;
//             match *child {
//                 ChildPtrMut::Node(node) => unsafe {
//                     let node = &mut *node;
//                     let (keys, values, _) = node.get_all_mut();
//                     let item = (&keys[*elem_idx], &mut values[*elem_idx]);
//
//                     while let Some(&(ChildPtrMut::Node(node), child_idx)) = self.left.last() {
//                         self.left.push((
//                             match (*node).children_mut() {
//                                 ChildrenSliceMut::Nodes(children) => {
//                                     ChildPtrMut::Node(&mut *children[child_idx])
//                                 }
//                                 ChildrenSliceMut::Leafs(children) => {
//                                     ChildPtrMut::Leaf(&mut *children[child_idx])
//                                 }
//                             },
//                             0,
//                         ));
//                     }
//                     Some(item)
//                 },
//                 ChildPtrMut::Leaf(leaf) => unsafe {
//                     let leaf = &mut *leaf;
//                     let (keys, values) = leaf.get_all_mut();
//                     let item = (&keys[*elem_idx], &mut values[*elem_idx]);
//
//                     while let Some(&(ref child, elem_idx)) = self.left.last() {
//                         if match child {
//                             ChildPtrMut::Leaf(leaf) => (**leaf).len(),
//                             ChildPtrMut::Node(node) => (**node).num_elements(),
//                         } <= elem_idx
//                         {
//                             self.left.pop();
//                         }
//                     }
//                     Some(item)
//                 },
//             }
//         } else {
//             None
//         }
//     }
//
//     fn size_hint(&self) -> (usize, Option<usize>) {
//         (self.len, Some(self.len))
//     }
//
//     fn count(self) -> usize {
//         self.len
//     }
// }
//
// impl<'a, K: Ord, V> DoubleEndedIterator for BTreeIterMut<'a, K, V> {
//     fn next_back(&mut self) -> Option<Self::Item> {
//         if 0 < self.len {
//             self.len -= 1;
//             let (child, elem_idx) = self.right.last_mut().unwrap();
//             *elem_idx -= 1;
//
//             match *child {
//                 ChildPtrMut::Node(node) => unsafe {
//                     let node = &mut *node;
//                     let (keys, values, _) = node.get_all_mut();
//                     let item = (&keys[*elem_idx], &mut values[*elem_idx]);
//
//                     while let Some(&(ChildPtrMut::Node(node), child_idx)) = self.right.last() {
//                         self.right.push(match (*node).children_mut() {
//                             ChildrenSliceMut::Nodes(children) => {
//                                 let child = &mut *children[child_idx];
//                                 (ChildPtrMut::Node(child), child.num_elements())
//                             }
//                             ChildrenSliceMut::Leafs(children) => {
//                                 let child = &mut *children[child_idx];
//                                 (ChildPtrMut::Leaf(child), child.len())
//                             }
//                         });
//                     }
//                     Some(item)
//                 },
//                 ChildPtrMut::Leaf(leaf) => unsafe {
//                     let leaf = &mut *leaf;
//                     let (keys, values) = leaf.get_all_mut();
//                     let item = (&keys[*elem_idx], &mut values[*elem_idx]);
//
//                     while let Some(&(_, elem_idx)) = self.right.last() {
//                         if elem_idx == 0 {
//                             self.right.pop();
//                         }
//                     }
//                     Some(item)
//                 },
//             }
//         } else {
//             None
//         }
//     }
// }
//
// impl<'a, K: Ord, V> ExactSizeIterator for BTreeIterMut<'a, K, V> {
//     fn len(&self) -> usize {
//         self.len
//     }
// }
