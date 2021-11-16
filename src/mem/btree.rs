use super::SlabAllocator;
use bitflags::bitflags;
use core::borrow::Borrow;
use core::mem::{self, MaybeUninit};
use core::ptr::{self, NonNull};
use core::slice;

const B: usize = 6;

const MIN_NUM_ELEMENTS: usize = B - 1;
const MAX_NUM_ELEMENTS: usize = 2 * B - 1;
const MIN_NUM_CHILDREN: usize = B;
const MAX_NUM_CHILDREN: usize = 2 * B;

bitflags! {
    struct LeafMetadata: u32 {}
    struct NodeMetadata: u32 {
        const NODE_CHILDREN = 1;
    }
}

#[repr(C)]
union ChildUnion<K, V> {
    node: NonNull<Node<K, V>>,
    leaf: NonNull<Leaf<K, V>>,
}

#[repr(C)]
enum Child<K, V> {
    Node(NonNull<Node<K, V>>),
    Leaf(NonNull<Leaf<K, V>>),
}

#[repr(C)]
struct Leaf<K, V> {
    keys: [MaybeUninit<K>; MAX_NUM_ELEMENTS],
    len: u32,
    metadata: LeafMetadata,
    values: [MaybeUninit<V>; MAX_NUM_ELEMENTS],
}

#[repr(C)]
struct Node<K, V> {
    keys: [MaybeUninit<K>; MAX_NUM_ELEMENTS],
    len: u32,
    metadata: NodeMetadata,
    children: [MaybeUninit<ChildUnion<K, V>>; MAX_NUM_CHILDREN],
    values: [MaybeUninit<V>; MAX_NUM_ELEMENTS],
}

impl<K: Ord, V> Leaf<K, V> {
    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub unsafe fn set_len(&mut self, len: usize) {
        self.len = len as _;
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.len() == MAX_NUM_ELEMENTS
    }

    #[inline]
    pub fn keys(&self) -> &[K] {
        unsafe { slice::from_raw_parts(self.keys.as_ptr() as _, self.len()) }
    }

    #[inline]
    pub fn keys_mut(&mut self) -> &mut [K] {
        unsafe { slice::from_raw_parts_mut(self.keys.as_mut_ptr() as _, self.len()) }
    }

    #[inline]
    pub fn values(&self) -> &[V] {
        unsafe { slice::from_raw_parts(self.values.as_ptr() as _, self.len()) }
    }

    #[inline]
    pub fn values_mut(&mut self) -> &mut [V] {
        unsafe { slice::from_raw_parts_mut(self.values.as_mut_ptr() as _, self.len()) }
    }

    #[inline]
    pub fn all_mut(&mut self) -> (&mut [K], &mut [V]) {
        (
            unsafe { slice::from_raw_parts_mut(self.keys.as_mut_ptr() as _, self.len()) },
            unsafe { slice::from_raw_parts_mut(self.values.as_mut_ptr() as _, self.len()) },
        )
    }

    pub fn search<Q>(&self, key: &Q) -> Result<usize, usize>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        for i in 0..self.len() {
            let ord = key.cmp(unsafe { self.keys[i].assume_init_ref().borrow() });
            if ord.is_eq() {
                return Ok(i);
            }
            if ord.is_lt() {
                return Err(i);
            }
        }
        Err(self.len())
    }

    pub fn insert(&mut self, index: usize, key: K, value: V) -> Option<(K, V)> {
        unsafe {
            if self.len() == MAX_NUM_ELEMENTS {
                Some((key, value))
            } else {
                ptr::copy(
                    self.keys.as_ptr().add(index),
                    self.keys.as_mut_ptr().add(index + 1),
                    self.len(),
                );
                ptr::copy(
                    self.values.as_ptr().add(index),
                    self.values.as_mut_ptr().add(index + 1),
                    self.len() - index,
                );
                self.keys[index].write(key);
                self.values[index].write(value);
                self.len += 1;

                None
            }
        }
    }

    pub fn insert_split<'a>(
        &mut self,
        index: usize,
        key: K,
        value: V,
        right: &'a mut MaybeUninit<Self>,
    ) -> (K, V, &'a mut Self) {
        assert_eq!(self.len(), MAX_NUM_ELEMENTS);

        unsafe {
            if index <= MIN_NUM_ELEMENTS + 1 {
                let right = right.write(Self {
                    keys: MaybeUninit::uninit().assume_init(),
                    values: MaybeUninit::uninit().assume_init(),
                    metadata: self.metadata,
                    len: MIN_NUM_ELEMENTS as _,
                });
                self.set_len(MIN_NUM_ELEMENTS + 1);
                ptr::copy_nonoverlapping(
                    self.keys.as_ptr().add(self.len()),
                    right.keys.as_mut_ptr(),
                    MIN_NUM_ELEMENTS,
                );
                ptr::copy_nonoverlapping(
                    self.values.as_ptr().add(self.len()),
                    right.values.as_mut_ptr(),
                    MIN_NUM_ELEMENTS,
                );

                if index == MIN_NUM_ELEMENTS + 1 {
                    (key, value, right)
                } else {
                    let (sep_key, sep_value) = self.pop_back().unwrap();
                    self.insert(index, key, value);

                    (sep_key, sep_value, right)
                }
            } else {
                let right = right.write(Self {
                    keys: MaybeUninit::uninit().assume_init(),
                    values: MaybeUninit::uninit().assume_init(),
                    metadata: self.metadata,
                    len: MIN_NUM_ELEMENTS as _,
                });
                self.set_len(MIN_NUM_ELEMENTS + 2);
                ptr::copy_nonoverlapping(
                    self.keys.as_ptr().add(self.len()),
                    right.keys.as_mut_ptr(),
                    MIN_NUM_ELEMENTS - 1,
                );
                ptr::copy_nonoverlapping(
                    self.values.as_ptr().add(self.len()),
                    right.values.as_mut_ptr(),
                    MIN_NUM_ELEMENTS - 1,
                );

                let (sep_key, sep_value) = self.pop_back().unwrap();
                right.insert(index - MIN_NUM_ELEMENTS - 2, key, value);

                (sep_key, sep_value, right)
            }
        }
    }

    /// Inserts the key and value to the correct position in the leaf.
    /// Returns the key and value in case the leaf is full.
    ///
    /// O(n)
    pub fn insert_overflow_back(&mut self, index: usize, key: K, value: V) -> (K, V) {
        assert_eq!(self.len(), MAX_NUM_ELEMENTS);
        if index == MAX_NUM_ELEMENTS {
            (key, value)
        } else {
            unsafe {
                let overflow = (
                    ptr::read(self.keys[MAX_NUM_ELEMENTS - 1].as_ptr()),
                    ptr::read(self.values[MAX_NUM_ELEMENTS - 1].as_ptr()),
                );
                ptr::copy(
                    self.keys.as_ptr().add(index),
                    self.keys.as_mut_ptr().add(index + 1),
                    MAX_NUM_ELEMENTS - index - 1,
                );
                ptr::copy(
                    self.values.as_ptr().add(index),
                    self.values.as_mut_ptr().add(index + 1),
                    MAX_NUM_ELEMENTS - index - 1,
                );
                self.keys[index].write(key);
                self.values[index].write(value);
                overflow
            }
        }
    }

    /// Inserts the key and value to the correct position in the leaf.
    ///
    /// O(n)
    pub fn insert_overflow_front(&mut self, index: usize, key: K, value: V) -> (K, V) {
        assert_eq!(self.len(), MAX_NUM_ELEMENTS);

        if index == 0 {
            (key, value)
        } else {
            unsafe {
                let overflow = (
                    ptr::read(self.keys[0].as_ptr()),
                    ptr::read(self.values[0].as_ptr()),
                );
                ptr::copy(self.keys.as_ptr().add(1), self.keys.as_mut_ptr(), index);
                ptr::copy(self.values.as_ptr().add(1), self.values.as_mut_ptr(), index);
                self.keys[index].write(key);
                self.values[index].write(value);
                overflow
            }
        }
    }

    /// Pushes the key and value to the front of the leaf.
    /// Returns the key and value in case the leaf is full.
    ///
    /// O(n)
    pub fn push_front(&mut self, key: K, value: V) -> Option<(K, V)> {
        // assert!(&key < unsafe { self.keys[0].assume_init_ref() });
        if self.len() == MAX_NUM_ELEMENTS {
            Some((key, value))
        } else {
            unsafe {
                ptr::copy(
                    self.keys.as_ptr(),
                    self.keys.as_mut_ptr().add(1),
                    self.len(),
                );
                ptr::copy(
                    self.values.as_ptr(),
                    self.values.as_mut_ptr().add(1),
                    self.len(),
                );
                self.keys[0].write(key);
                self.values[0].write(value);
                self.len += 1;

                None
            }
        }
    }

    /// Pushes the key and value to the back of the leaf.
    /// Returns the key and value in case the leaf is full.
    ///
    /// O(1)
    pub fn push_back(&mut self, key: K, value: V) -> Option<(K, V)> {
        // assert!(unsafe { self.keys[self.len() - 1].assume_init_ref() } < &key);
        if self.len() == MAX_NUM_ELEMENTS {
            Some((key, value))
        } else {
            self.keys[self.len()].write(key);
            self.values[self.len()].write(value);
            self.len += 1;

            None
        }
    }

    /// Pops the first key and value in the leaf.
    ///
    /// O(n)
    pub fn pop_front(&mut self) -> Option<(K, V)> {
        if self.len() == 0 {
            None
        } else {
            unsafe {
                self.len -= 1;
                let overflow = Some((
                    ptr::read(self.keys[0].as_ptr()),
                    ptr::read(self.values[0].as_ptr()),
                ));
                ptr::copy(
                    self.keys.as_ptr().add(1),
                    self.keys.as_mut_ptr(),
                    self.len(),
                );
                ptr::copy(
                    self.values.as_ptr().add(1),
                    self.values.as_mut_ptr(),
                    self.len(),
                );
                overflow
            }
        }
    }

    /// Pops the last key and value in the leaf.
    ///
    /// O(1)
    pub fn pop_back(&mut self) -> Option<(K, V)> {
        if self.len() == 0 {
            None
        } else {
            unsafe {
                self.len -= 1;
                let overflow = Some((
                    ptr::read(self.keys[self.len()].as_ptr()),
                    ptr::read(self.values[self.len()].as_ptr()),
                ));
                overflow
            }
        }
    }
}

// TODO: Convert all of the leaf code to node.
impl<K: Ord, V> Node<K, V> {
    pub fn new(slf: &mut MaybeUninit<Self>, left_child: Child<K, V>) -> &mut Self {
        unsafe {
            let slf = slf.write(Self {
                keys: MaybeUninit::uninit().assume_init(),
                values: MaybeUninit::uninit().assume_init(),
                children: MaybeUninit::uninit().assume_init(),
                len: 0,
                metadata: if matches!(left_child, Child::Node(_)) {
                    NodeMetadata::NODE_CHILDREN
                } else {
                    NodeMetadata::empty()
                },
            });
            slf.children[0].write(match left_child {
                Child::Node(left_child) => ChildUnion { node: left_child },
                Child::Leaf(left_child) => ChildUnion { leaf: left_child },
            });

            slf
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.len as usize
    }

    #[inline]
    pub unsafe fn set_len(&mut self, len: usize) {
        self.len = len as _;
    }

    #[inline]
    pub fn is_full(&self) -> bool {
        self.len() == MAX_NUM_ELEMENTS
    }

    #[inline]
    pub fn keys(&self) -> &[K] {
        unsafe { slice::from_raw_parts(self.keys.as_ptr() as _, self.len()) }
    }

    #[inline]
    pub fn keys_mut(&mut self) -> &mut [K] {
        unsafe { slice::from_raw_parts_mut(self.keys.as_mut_ptr() as _, self.len()) }
    }

    #[inline]
    pub fn values(&self) -> &[V] {
        unsafe { slice::from_raw_parts(self.values.as_ptr() as _, self.len()) }
    }

    #[inline]
    pub fn values_mut(&mut self) -> &mut [V] {
        unsafe { slice::from_raw_parts_mut(self.values.as_mut_ptr() as _, self.len()) }
    }

    #[inline]
    pub fn keys_values_mut(&mut self) -> (&mut [K], &mut [V]) {
        (
            unsafe { slice::from_raw_parts_mut(self.keys.as_mut_ptr() as _, self.len()) },
            unsafe { slice::from_raw_parts_mut(self.values.as_mut_ptr() as _, self.len()) },
        )
    }

    pub fn search<Q>(&self, key: &Q) -> Result<usize, usize>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        for i in 0..self.len() {
            let ord = key.cmp(unsafe { self.keys[i].assume_init_ref().borrow() });
            if ord.is_eq() {
                return Ok(i);
            }
            if ord.is_lt() {
                return Err(i);
            }
        }
        Err(self.len())
    }

    pub fn insert(
        &mut self,
        index: usize,
        key: K,
        value: V,
        right_child: ChildUnion<K, V>,
    ) -> Option<(K, V, ChildUnion<K, V>)> {
        unsafe {
            if self.len() == MAX_NUM_ELEMENTS {
                Some((key, value, right_child))
            } else {
                ptr::copy(
                    self.keys.as_ptr().add(index),
                    self.keys.as_mut_ptr().add(index + 1),
                    self.len(),
                );
                ptr::copy(
                    self.values.as_ptr().add(index),
                    self.values.as_mut_ptr().add(index + 1),
                    self.len() - index,
                );
                ptr::copy(
                    self.children.as_ptr().add(index + 1),
                    self.children.as_mut_ptr().add(index + 2),
                    self.len() - index,
                );
                self.keys[index].write(key);
                self.values[index].write(value);
                self.children[index + 1].write(right_child);
                self.len += 1;

                None
            }
        }
    }

    pub fn insert_split<'a>(
        &mut self,
        index: usize,
        key: K,
        value: V,
        right_child: ChildUnion<K, V>,
        right: &'a mut MaybeUninit<Self>,
    ) -> (K, V, &'a mut Self) {
        assert_eq!(self.len(), MAX_NUM_ELEMENTS);

        unsafe {
            if index <= MIN_NUM_ELEMENTS + 1 {
                let right = right.write(Self {
                    keys: MaybeUninit::uninit().assume_init(),
                    values: MaybeUninit::uninit().assume_init(),
                    children: MaybeUninit::uninit().assume_init(),
                    metadata: self.metadata,
                    len: MIN_NUM_ELEMENTS as _,
                });
                self.set_len(MIN_NUM_ELEMENTS + 1);
                ptr::copy_nonoverlapping(
                    self.keys.as_ptr().add(self.len()),
                    right.keys.as_mut_ptr(),
                    MIN_NUM_ELEMENTS,
                );
                ptr::copy_nonoverlapping(
                    self.values.as_ptr().add(self.len()),
                    right.values.as_mut_ptr(),
                    MIN_NUM_ELEMENTS,
                );
                ptr::copy_nonoverlapping(
                    self.children.as_ptr().add(self.len()),
                    right.children.as_mut_ptr(),
                    MIN_NUM_ELEMENTS + 1,
                );

                if index == MIN_NUM_ELEMENTS + 1 {
                    right.children[0].write(right_child);
                    (key, value, right)
                } else {
                    let (sep_key, sep_value, _) = self.pop_back().unwrap();
                    self.insert(index, key, value, right_child);

                    (sep_key, sep_value, right)
                }
            } else {
                let right = right.write(Self {
                    keys: MaybeUninit::uninit().assume_init(),
                    values: MaybeUninit::uninit().assume_init(),
                    children: MaybeUninit::uninit().assume_init(),
                    metadata: self.metadata,
                    len: MIN_NUM_ELEMENTS as _,
                });
                self.set_len(MIN_NUM_ELEMENTS + 2);
                ptr::copy_nonoverlapping(
                    self.keys.as_ptr().add(self.len()),
                    right.keys.as_mut_ptr(),
                    MIN_NUM_ELEMENTS - 1,
                );
                ptr::copy_nonoverlapping(
                    self.values.as_ptr().add(self.len()),
                    right.values.as_mut_ptr(),
                    MIN_NUM_ELEMENTS - 1,
                );
                ptr::copy_nonoverlapping(
                    self.children.as_ptr().add(self.len()),
                    right.children.as_mut_ptr(),
                    MIN_NUM_ELEMENTS,
                );

                let (sep_key, sep_value, _) = self.pop_back().unwrap();
                right.insert(index - MIN_NUM_ELEMENTS - 2, key, value, right_child);

                (sep_key, sep_value, right)
            }
        }
    }

    /// Inserts the key and value to the correct position in the leaf.
    /// Returns the key and value in case the leaf is full.
    ///
    /// O(n)
    pub fn insert_overflow_back(
        &mut self,
        index: usize,
        key: K,
        value: V,
        right_child: ChildUnion<K, V>,
    ) -> (K, V, ChildUnion<K, V>) {
        assert_eq!(self.len(), MAX_NUM_ELEMENTS);
        if index == MAX_NUM_ELEMENTS {
            (key, value, right_child)
        } else {
            unsafe {
                let overflow = (
                    ptr::read(self.keys[MAX_NUM_ELEMENTS - 1].as_ptr()),
                    ptr::read(self.values[MAX_NUM_ELEMENTS - 1].as_ptr()),
                    ptr::read(self.children[MAX_NUM_CHILDREN - 1].as_ptr()),
                );
                ptr::copy(
                    self.keys.as_ptr().add(index),
                    self.keys.as_mut_ptr().add(index + 1),
                    MAX_NUM_ELEMENTS - index - 1,
                );
                ptr::copy(
                    self.values.as_ptr().add(index),
                    self.values.as_mut_ptr().add(index + 1),
                    MAX_NUM_ELEMENTS - index - 1,
                );
                ptr::copy(
                    self.values.as_ptr().add(index + 1),
                    self.values.as_mut_ptr().add(index + 2),
                    MAX_NUM_CHILDREN - index - 2,
                );
                self.keys[index].write(key);
                self.values[index].write(value);
                self.children[index + 1].write(right_child);
                overflow
            }
        }
    }

    /// Inserts the key and value to the correct position in the leaf.
    ///
    /// O(n)
    pub fn insert_overflow_front(
        &mut self,
        index: usize,
        key: K,
        value: V,
        right_child: ChildUnion<K, V>,
    ) -> (K, V, ChildUnion<K, V>) {
        assert_eq!(self.len(), MAX_NUM_ELEMENTS);

        unsafe {
            if index == 0 {
                (
                    key,
                    value,
                    mem::replace(self.children[0].assume_init_mut(), right_child),
                )
            } else {
                let overflow = (
                    ptr::read(self.keys[0].as_ptr()),
                    ptr::read(self.values[0].as_ptr()),
                    ptr::read(self.children[0].as_ptr()),
                );
                ptr::copy(self.keys.as_ptr().add(1), self.keys.as_mut_ptr(), index);
                ptr::copy(self.values.as_ptr().add(1), self.values.as_mut_ptr(), index);
                ptr::copy(
                    self.children.as_ptr().add(1),
                    self.children.as_mut_ptr(),
                    index + 1,
                );
                self.keys[index].write(key);
                self.values[index].write(value);
                self.children[index + 1].write(right_child);
                overflow
            }
        }
    }

    /// Pushes the key and value to the front of the leaf.
    /// Returns the key and value in case the leaf is full.
    ///
    /// O(n)
    pub fn push_front(
        &mut self,
        key: K,
        value: V,
        left_child: ChildUnion<K, V>,
    ) -> Option<(K, V, ChildUnion<K, V>)> {
        // assert!(&key < unsafe { self.keys[0].assume_init_ref() });
        if self.len() == MAX_NUM_ELEMENTS {
            Some((key, value, left_child))
        } else {
            unsafe {
                ptr::copy(
                    self.keys.as_ptr(),
                    self.keys.as_mut_ptr().add(1),
                    self.len(),
                );
                ptr::copy(
                    self.values.as_ptr(),
                    self.values.as_mut_ptr().add(1),
                    self.len(),
                );
                ptr::copy(
                    self.children.as_ptr(),
                    self.children.as_mut_ptr().add(1),
                    self.len() + 1,
                );
                self.keys[0].write(key);
                self.values[0].write(value);
                self.children[0].write(left_child);
                self.len += 1;

                None
            }
        }
    }

    /// Pushes the key and value to the back of the leaf.
    /// Returns the key and value in case the leaf is full.
    ///
    /// O(1)
    pub fn push_back(
        &mut self,
        key: K,
        value: V,
        right_child: ChildUnion<K, V>,
    ) -> Option<(K, V, ChildUnion<K, V>)> {
        // assert!(unsafe { self.keys[self.len() - 1].assume_init_ref() } < &key);
        if self.len() == MAX_NUM_ELEMENTS {
            Some((key, value, right_child))
        } else {
            self.keys[self.len()].write(key);
            self.values[self.len()].write(value);
            self.children[self.len() + 1].write(right_child);
            self.len += 1;

            None
        }
    }

    /// Pops the first key and value in the leaf.
    ///
    /// O(n)
    pub fn pop_front(&mut self) -> Option<(K, V, ChildUnion<K, V>)> {
        if self.len() == 0 {
            None
        } else {
            unsafe {
                self.len -= 1;
                let overflow = Some((
                    ptr::read(self.keys[0].as_ptr()),
                    ptr::read(self.values[0].as_ptr()),
                    ptr::read(self.children[0].as_ptr()),
                ));
                ptr::copy(
                    self.keys.as_ptr().add(1),
                    self.keys.as_mut_ptr(),
                    self.len(),
                );
                ptr::copy(
                    self.values.as_ptr().add(1),
                    self.values.as_mut_ptr(),
                    self.len(),
                );
                ptr::copy(
                    self.children.as_ptr().add(1),
                    self.children.as_mut_ptr(),
                    self.len() + 1,
                );
                overflow
            }
        }
    }

    /// Pops the last key and value in the leaf.
    ///
    /// O(1)
    pub fn pop_back(&mut self) -> Option<(K, V, ChildUnion<K, V>)> {
        if self.len() == 0 {
            None
        } else {
            unsafe {
                self.len -= 1;
                let overflow = Some((
                    ptr::read(self.keys[self.len()].as_ptr()),
                    ptr::read(self.values[self.len()].as_ptr()),
                    ptr::read(self.children[self.len() + 1].as_ptr()),
                    // {
                    //     let child = ptr::read(self.children[self.len() + 1].as_ptr());

                    //     if self.metadata.contains(NodeMetadata::NODE_CHILDREN) {
                    //         Child::Node(child.node)
                    //     } else {
                    //         Child::Leaf(child.leaf)
                    //     }
                    // },
                ));
                // let x = &mut 1;
                // let y = x;
                // *x = 2;
                // *y = 3;
                overflow
            }
        }
    }
}

enum Root<K, V> {
    Node(NonNull<Node<K, V>>),
    Leaf(NonNull<Leaf<K, V>>),
}

pub struct BTreeMap<K, V> {
    node_alloc: SlabAllocator<Node<K, V>>,
    leaf_alloc: SlabAllocator<Leaf<K, V>>,

    root: Root<K, V>,
}

impl<K: Ord, V> BTreeMap<K, V> {
    pub fn insert(&mut self, key: K, value: V) -> Option<(K, V)> {
        unsafe {
            match self.root {
                Root::Node(mut node) => {
                    todo!()
                }
                Root::Leaf(mut root) => {
                    let root = root.as_mut();
                    match root.search(&key) {
                        Ok(idx) => Some((key, mem::replace(&mut root.values_mut()[idx], value))),
                        Err(idx) => {
                            if root.is_full() {
                                let right = &mut *self.leaf_alloc.malloc().unwrap().cast().as_ptr();
                                let (sep_key, sep_value, right) =
                                    root.insert_split(idx, key, value, right);

                                let new_root = Node::new(
                                    &mut *self.node_alloc.malloc().unwrap().cast().as_ptr(),
                                    Child::Leaf(NonNull::new_unchecked(root)),
                                );
                                new_root.push_back(
                                    sep_key,
                                    sep_value,
                                    ChildUnion {
                                        leaf: NonNull::new_unchecked(right),
                                    },
                                );

                                self.root = Root::Node(NonNull::new_unchecked(new_root));

                                None
                            } else {
                                root.insert(idx, key, value);
                                None
                            }
                        }
                    }
                }
            }
        }
    }
}
