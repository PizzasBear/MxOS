use core::marker::PhantomData;
use core::mem::size_of;
use core::ptr;

/// A slab allocator, that allocates only type T. It needs a page allocator, but it never
/// deallocates.
#[derive(Debug)]
pub struct SlabAllocator<T> {
    free_size: usize,
    free_list: ptr::NonNull<SlabFreeList>,
    _phantom: PhantomData<T>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(16))]
struct SlabFreeList {
    size: usize,
    next: Option<ptr::NonNull<SlabFreeList>>,
}

impl<T: Sized> SlabAllocator<T> {
    const SLAB_SIZE: usize = size_of::<T>();

    /// Creates a new slab allocator from a page allocator.
    ///
    /// # Safety
    /// `chunk_addr` has to be a pointer to a chunk of 2 MiB.
    pub fn new(chunk: &'static mut [u8]) -> Self {
        unsafe {
            assert_eq!(size_of::<SlabFreeList>(), 16);

            assert!(
                16 <= Self::SLAB_SIZE,
                "Slab allocator's type T size, {} bytes, is smaller than 16 bytes",
                Self::SLAB_SIZE,
            );
            assert_eq!(Self::SLAB_SIZE & 0xf, 0);

            let free_size = chunk.len() - chunk.len() % Self::SLAB_SIZE;
            Self {
                free_size,
                free_list: {
                    let mut free_list = ptr::NonNull::new(chunk.as_mut_ptr() as _).unwrap();
                    *free_list.as_mut() = SlabFreeList {
                        size: free_size,
                        next: None,
                    };
                    free_list
                },
                _phantom: PhantomData,
            }
        }
    }

    /// Allocates a pointer to `T`.
    pub fn add_chunk(&mut self, chunk: &'static mut [u8]) {
        unsafe {
            let alloc_size = chunk.len() - chunk.len() % Self::SLAB_SIZE;

            self.free_size += alloc_size;
            let mut free_list = ptr::NonNull::new(chunk.as_mut_ptr() as _).unwrap();
            *free_list.as_mut() = SlabFreeList {
                size: alloc_size,
                next: Some(self.free_list),
            };
            self.free_list = free_list;
        }
    }

    /// Returns true if the allocator needs a new chunk. To add the new chunk call `add_chunk`.
    pub fn needs_new_chunk(&self) -> bool {
        self.free_size < 64 * Self::SLAB_SIZE
    }

    /// Allocates a pointer to `T`. Make sure to not leak this memory
    pub fn malloc(&mut self) -> Option<ptr::NonNull<T>> {
        unsafe {
            let SlabFreeList { size, next } = *self.free_list.as_mut();
            if Self::SLAB_SIZE < size {
                let ptr = ptr::NonNull::new(self.free_list.as_ptr() as _)?;
                self.free_list =
                    ptr::NonNull::new((self.free_list.as_ptr() as usize + Self::SLAB_SIZE) as _)
                        .unwrap();
                self.free_list.as_mut().size -= Self::SLAB_SIZE;
                self.free_size -= Self::SLAB_SIZE;

                Some(ptr)
            } else if Self::SLAB_SIZE == size {
                let ptr = ptr::NonNull::new(self.free_list.as_ptr() as _)?;
                self.free_list = next?;
                self.free_size -= Self::SLAB_SIZE;

                Some(ptr)
            } else {
                log::error!("Slab allocator free area is too small");
                self.free_list = next?;
                self.free_size -= size;

                self.malloc()
            }
        }
    }

    /// Deallocates a pointer to `T`;
    ///
    /// # Safety
    /// `ptr` must point to a value allocated by this slab allocator
    pub unsafe fn free(&mut self, ptr: ptr::NonNull<T>) {
        let free_list = self.free_list;
        self.free_list = ptr::NonNull::new(ptr.as_ptr() as _).unwrap();
        *self.free_list.as_mut() = SlabFreeList {
            size: Self::SLAB_SIZE,
            next: Some(free_list),
        };
        self.free_size += Self::SLAB_SIZE;
    }
}

// pub struct LockedSlabAllocator<T>(spin::Mutex<SlabAllocator<T>>);
//
// pub struct SlabBox<'a, T> {
//     data: &'a mut T,
//     alloc: &'a LockedSlabAllocator<T>,
// }
//
// impl<T: Sized> LockedSlabAllocator<T> {
//     /// Creates a new slab allocator from a page allocator.
//     ///
//     /// # Safety
//     /// `chunk_addr` has to be a pointer to a chunk of 2 MiB.
//     pub fn new(chunk: &'static mut [u8]) -> Self {
//         Self(spin::Mutex::new(SlabAllocator::new(chunk)))
//     }
//
//     /// Allocates a pointer to `T`.
//     pub fn add_chunk(&self, chunk: &'static mut [u8]) {
//         self.0.lock().add_chunk(chunk);
//     }
//
//     /// Returns true if the allocator needs a new chunk. To add the new chunk call `add_chunk`.
//     pub fn needs_new_chunk(&self) -> bool {
//         self.0.lock().needs_new_chunk()
//     }
//
//     /// Allocates a pointer to `T`. Make sure to not leak this memory
//     pub fn malloc(&self, data: T) -> Option<SlabBox<T>> {
//         unsafe {
//             let mut ptr = self.0.lock().malloc()?;
//             *ptr.as_mut() = data;
//             Some(SlabBox {
//                 data: ptr.as_mut(),
//                 alloc: self,
//             })
//         }
//     }
// }
//
// impl<'a, T> Drop for SlabBox<'a, T> {
//     fn drop(&mut self) {
//         unsafe {
//             self.alloc
//                 .0
//                 .lock()
//                 .free(ptr::NonNull::new(self.data).unwrap());
//         }
//     }
// }
