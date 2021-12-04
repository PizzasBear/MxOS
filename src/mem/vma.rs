use super::BTree;
use core::ptr::NonNull;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
struct SizeFirstPtrSecond {
    size: usize,
    ptr: NonNull<u8>,
}

pub struct VirtualMemoryAllocator {
    best_fit_tree: BTree<SizeFirstPtrSecond, ()>,
    merge_tree: BTree<NonNull<u8>, usize>,
}

impl VirtualMemoryAllocator {
    pub fn new(free_areas: &[(NonNull<u8>, usize)], chunk: &'static mut [u8]) -> Self {
        let (chunk0, chunk1) = chunk.split_at_mut(chunk.len() / 2);

        let mut best_fit_tree = BTree::new(chunk0);
        let mut merge_tree = BTree::new(chunk1);

        for &(ptr, size) in free_areas.iter() {
            assert!(best_fit_tree
                .insert(SizeFirstPtrSecond { size, ptr }, ())
                .is_none());
            assert!(merge_tree.insert(ptr, size).is_none());
        }

        Self {
            best_fit_tree,
            merge_tree,
        }
    }

    pub fn needs_new_chunk(&self) -> bool {
        self.best_fit_tree.needs_new_chunk() || self.merge_tree.needs_new_chunk()
    }

    pub fn add_chunk(&mut self, chunk: &'static mut [u8]) {
        match (
            self.best_fit_tree.needs_new_chunk(),
            self.merge_tree.needs_new_chunk(),
        ) {
            (true, false) => {
                self.best_fit_tree.add_chunk(chunk);
            }
            (false, true) => {
                self.merge_tree.add_chunk(chunk);
            }
            (true, true) | (false, false) => {
                let (chunk0, chunk1) = chunk.split_at_mut(chunk.len() / 2);

                self.best_fit_tree.add_chunk(chunk0);
                self.merge_tree.add_chunk(chunk1);
            }
        }
    }

    pub fn alloc(&mut self, alloc_size: usize) -> (NonNull<u8>, usize) {
        // Align to 2MiB
        let alloc_size = alloc_size + 0x1fffff & !0x1fffff;

        let SizeFirstPtrSecond {
            ptr,
            size: area_size,
        } = match self.best_fit_tree.get_entry(&SizeFirstPtrSecond {
            ptr: NonNull::dangling(),
            size: alloc_size,
        }) {
            Ok(_) => unreachable!(),
            Err(mut entry) => {
                if alloc_size < entry.key().size {
                    *entry.key()
                } else {
                    assert!(entry.next());
                    assert!(alloc_size < entry.key().size);
                    *entry.key()
                }
            }
        };

        self.best_fit_tree.remove(&SizeFirstPtrSecond {
            ptr,
            size: area_size,
        });
        self.merge_tree.remove(&ptr);

        if alloc_size < area_size {
            let new_ptr = unsafe { NonNull::new(ptr.as_ptr().add(alloc_size)).unwrap() };
            let size = area_size - alloc_size;

            assert!(self
                .best_fit_tree
                .insert(SizeFirstPtrSecond { ptr: new_ptr, size }, ())
                .is_none());
            assert!(self.merge_tree.insert(new_ptr, size).is_none());
        } else {
            assert_eq!(alloc_size, area_size);
        }

        (ptr, alloc_size)
    }

    pub fn free(&mut self, mut ptr: NonNull<u8>, mut size: usize) {
        assert_eq!(size & 0x1fffff, 0);

        let entry = self.merge_tree.get_entry(&ptr).unwrap_err();
        if ptr < *entry.key() {
            let end_ptr = unsafe { NonNull::new(ptr.as_ptr().add(size)).unwrap() };

            let mut entry = if end_ptr == *entry.key() {
                size += *entry.value();
                drop(entry);

                let (entry_ptr, entry_size) = self.merge_tree.remove(&end_ptr).unwrap();
                self.best_fit_tree
                    .remove(&SizeFirstPtrSecond {
                        ptr: entry_ptr,
                        size: entry_size,
                    })
                    .unwrap();

                self.merge_tree.get_entry(&ptr).unwrap_err()
            } else {
                entry
            };

            if *entry.key() < ptr || entry.prev() {
                let entry_end_ptr =
                    unsafe { NonNull::new(entry.key().as_ptr().add(*entry.value())).unwrap() };

                if entry_end_ptr == ptr {
                    ptr = *entry.key();
                    size += *entry.value();
                    drop(entry);

                    let (entry_ptr, entry_size) = self.merge_tree.remove(&ptr).unwrap();
                    self.best_fit_tree
                        .remove(&SizeFirstPtrSecond {
                            ptr: entry_ptr,
                            size: entry_size,
                        })
                        .unwrap();
                }
            }
        } else {
            let entry_end_ptr =
                unsafe { NonNull::new(entry.key().as_ptr().add(*entry.value())).unwrap() };
            let mut entry = if ptr == entry_end_ptr {
                ptr = *entry.key();
                size += *entry.value();
                drop(entry);

                let (entry_ptr, entry_size) = self.merge_tree.remove(&ptr).unwrap();
                self.best_fit_tree
                    .remove(&SizeFirstPtrSecond {
                        ptr: entry_ptr,
                        size: entry_size,
                    })
                    .unwrap();

                self.merge_tree.get_entry(&ptr).unwrap_err()
            } else {
                entry
            };

            if ptr < *entry.key() || entry.next() {
                let end_ptr = unsafe { NonNull::new(ptr.as_ptr().add(size)).unwrap() };

                if end_ptr == *entry.key() {
                    size += *entry.value();
                    drop(entry);

                    let (entry_ptr, entry_size) = self.merge_tree.remove(&end_ptr).unwrap();
                    self.best_fit_tree
                        .remove(&SizeFirstPtrSecond {
                            ptr: entry_ptr,
                            size: entry_size,
                        })
                        .unwrap();
                }
            }
        }

        self.merge_tree.insert(ptr, size);
        self.best_fit_tree
            .insert(SizeFirstPtrSecond { ptr, size }, ());
    }
}
