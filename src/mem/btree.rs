use super::SlabAllocator;
use bitflags::bitflags;
use core::mem::MaybeUninit;
use core::ptr::NonNull;

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
union Children<K, V> {
    nodes: [NonNull<Node<K, V>>; MAX_NUM_CHILDREN],
    leafs: [NonNull<Leaf<K, V>>; MAX_NUM_CHILDREN],
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
    children: Children<K, V>,
    values: [MaybeUninit<V>; MAX_NUM_ELEMENTS],
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

impl<K: Ord> BTreeMap<K, V> {}
