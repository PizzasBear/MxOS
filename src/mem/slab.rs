use core::marker::PhantomData;
use core::mem::{self, size_of};
use core::{fmt, ops, ptr};

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

    /// Allocates a pointer to `T`. Make sure to not leak this memory.
    /// Using this function directly is not recommended, please use `SlabBox::<T>::new(slf, data)` instead.
    pub fn malloc(&mut self) -> Option<ptr::NonNull<T>> {
        unsafe {
            let SlabFreeList { size, next } = *self.free_list.as_mut();
            if Self::SLAB_SIZE < size {
                let ptr = ptr::NonNull::new(self.free_list.as_ptr() as _)?;
                self.free_list =
                    ptr::NonNull::new((self.free_list.as_ptr() as usize + Self::SLAB_SIZE) as _)
                        .unwrap();
                *self.free_list.as_mut() = SlabFreeList { size, next };
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
    /// `ptr` must point to a value allocated by a slab allocator.
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

/// Represents a box allocated by a slab allocator.
#[repr(transparent)]
pub struct SlabBox<T> {
    ptr: ptr::NonNull<T>,
    phantom: PhantomData<T>,
}

impl<T> SlabBox<T> {
    /// Allocates the box from the given slab allocator and moves x to it.
    #[inline]
    pub fn new(alloc: &mut SlabAllocator<T>, x: T) -> Self {
        unsafe {
            let ptr = alloc.malloc().expect("Failed to allocate");
            ptr.cast::<mem::MaybeUninit<T>>().as_mut().write(x);
            Self {
                ptr,
                phantom: PhantomData,
            }
        }
    }

    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self.ptr.as_ptr()
    }

    /// Frees the allocation with the given allocator. This allocator doesn't have to be the same
    /// allocator that was used to allocate this box, but it's not recommended to use a different
    /// allocator or multiple slab allocators of the same type in general.
    #[inline]
    pub fn free(self, alloc: &mut SlabAllocator<T>) {
        unsafe {
            let md = mem::ManuallyDrop::new(self);
            md.ptr.as_ptr().drop_in_place();
            alloc.free(md.ptr);
        }
    }

    /// Does the same thing as `free` but without dropping the data inside.
    #[inline]
    pub fn free_forget(self, alloc: &mut SlabAllocator<T>) {
        unsafe {
            let md = mem::ManuallyDrop::new(self);
            alloc.free(md.ptr);
        }
    }

    /// Does the same thing as `free` but moves the data and returns it.
    #[inline]
    pub fn free_move(self, alloc: &mut SlabAllocator<T>) -> T {
        let x;
        unsafe {
            let md = mem::ManuallyDrop::new(self);
            x = md.ptr.as_ptr().read();
            alloc.free(md.ptr);
        }
        x
    }

    /// Clones the box, does the same thing as
    /// ```
    /// // slf: SlabBox<T>
    /// // alloc: &mut SlabAllocator<T>
    /// SlabBox::new(alloc, slf.as_ref().clone())
    /// ```
    #[inline]
    pub fn clone(&self, alloc: &mut SlabAllocator<T>) -> Self
    where
        T: Clone,
    {
        Self::new(alloc, self.as_ref().clone())
    }
}

unsafe impl<T: Send> Send for SlabAllocator<T> {}
unsafe impl<T: Sync> Sync for SlabAllocator<T> {}

impl<T> AsRef<T> for SlabBox<T> {
    #[inline]
    fn as_ref(&self) -> &T {
        &*self
    }
}

impl<T> AsMut<T> for SlabBox<T> {
    #[inline]
    fn as_mut(&mut self) -> &mut T {
        &mut *self
    }
}

impl<T> ops::Deref for SlabBox<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T> ops::DerefMut for SlabBox<T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.ptr.as_mut() }
    }
}

impl<T> Drop for SlabBox<T> {
    #[inline]
    fn drop(&mut self) {
        panic!("A slab box was dropped resulting in a leak.");
    }
}

impl<T: fmt::Display> fmt::Display for SlabBox<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<T: fmt::Debug> fmt::Debug for SlabBox<T> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

unsafe impl<T: Send> Send for SlabBox<T> {}
unsafe impl<T: Sync> Sync for SlabBox<T> {}

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
