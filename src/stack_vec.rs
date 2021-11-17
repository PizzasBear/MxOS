use core::{
    fmt,
    mem::{self, ManuallyDrop, MaybeUninit},
    ops::{Bound, RangeBounds},
    ptr, slice,
};

trait MaybeUninitExt: Sized {
    type Item;

    fn uninit_array<const LEN: usize>() -> [Self; LEN];
    unsafe fn slice_assume_init_ref(slice: &[Self]) -> &[Self::Item];
    unsafe fn slice_assume_init_mut(slice: &mut [Self]) -> &mut [Self::Item];
}

impl<T> MaybeUninitExt for MaybeUninit<T> {
    type Item = T;

    fn uninit_array<const LEN: usize>() -> [Self; LEN] {
        // SAFETY: An uninitialized `[MaybeUninit<_>; LEN]` is valid.
        unsafe { MaybeUninit::<[Self; LEN]>::uninit().assume_init() }
    }

    unsafe fn slice_assume_init_ref(slice: &[Self]) -> &[T] {
        // SAFETY: casting slice to a `*const [T]` is safe since the caller guarantees that
        // `slice` is initialized, and`MaybeUninit` is guaranteed to have the same layout as `T`.
        // The pointer obtained is valid since it refers to memory owned by `slice` which is a
        // reference and thus guaranteed to be valid for reads.
        &*(slice as *const [Self] as *const [T])
    }

    /// Assuming all the elements are initialized, get a mutable slice to them.
    ///
    /// # Safety
    ///
    /// It is up to the caller to guarantee that the `MaybeUninit<T>` elements
    /// really are in an initialized state.
    /// Calling this when the content is not yet fully initialized causes undefined behavior.
    ///
    /// See [`assume_init_mut`] for more details and examples.
    ///
    /// [`assume_init_mut`]: MaybeUninit::assume_init_mut
    unsafe fn slice_assume_init_mut(slice: &mut [Self]) -> &mut [T] {
        // SAFETY: similar to safety notes for `slice_get_ref`, but we have a
        // mutable reference which is also guaranteed to be valid for writes.
        &mut *(slice as *mut [Self] as *mut [T])
    }
}

/// A `StackVec` that doesn't store it's own length.
/// Most of the operations are inline, that's because this struct is intended to be wrapped.
///
/// # SAFETY
/// Make sure the length isn't modified by anything other than `OuterLenStackVec` methods.
/// Initialize the length to 0.
#[repr(transparent)]
pub struct OuterLenStackVec<T, const N: usize> {
    _data: [MaybeUninit<T>; N],
}

pub struct OuterLenStackVecDrain<'a, T, const N: usize> {
    tail_start: usize,
    tail_len: usize,
    vec: ptr::NonNull<OuterLenStackVec<T, N>>,
    vec_len: &'a mut usize,
    iter: slice::Iter<'a, T>,
}

impl<T, const N: usize> OuterLenStackVec<T, N> {
    pub fn new() -> Self {
        Self {
            _data: MaybeUninitExt::uninit_array(),
        }
    }

    #[must_use]
    #[inline]
    pub unsafe fn push(&mut self, len: &mut usize, item: T) -> Option<T> {
        if *len == N {
            Some(item)
        } else {
            self._data[*len] = MaybeUninit::new(item);
            *len += 1;
            None
        }
    }

    #[must_use]
    #[inline]
    pub unsafe fn insert(&mut self, len: &mut usize, idx: usize, item: T) -> Option<T> {
        assert!(idx <= *len);

        if idx == N {
            Some(item)
        } else {
            let overflowed = if *len == N {
                Some(self._data[*len - 1].as_ptr().read())
            } else {
                *len += 1;
                None
            };

            core::ptr::copy(
                self._data.as_ptr().add(idx),
                self._data.as_mut_ptr().add(idx + 1),
                *len - idx - 1,
            );
            self._data[idx] = MaybeUninit::new(item);

            overflowed
        }
    }

    #[must_use]
    #[inline]
    pub unsafe fn pop(&mut self, len: &mut usize) -> Option<T> {
        if 0 < *len {
            *len -= 1;
            Some(self._data[*len].as_ptr().read())
        } else {
            None
        }
    }

    #[inline]
    pub unsafe fn remove(&mut self, len: &mut usize, idx: usize) -> T {
        assert!(idx < *len);
        *len -= 1;

        let item = self._data[idx].as_ptr().read();
        core::ptr::copy(
            self._data.as_ptr().add(idx + 1),
            self._data.as_mut_ptr().add(idx),
            *len - idx,
        );

        item
    }

    #[inline]
    pub unsafe fn split_at(&mut self, len: &mut usize, left_len: usize) -> StackVec<T, N> {
        assert!(left_len <= *len);

        let mut right = StackVec::new();
        right.set_len(*len - left_len);

        core::ptr::copy_nonoverlapping(
            self._data.as_ptr().add(left_len),
            right.data_mut().as_mut_ptr(),
            right._len,
        );
        *len = left_len;

        right
    }

    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        N
    }

    #[inline]
    pub unsafe fn as_slice(&self, len: usize) -> &[T] {
        MaybeUninitExt::slice_assume_init_ref(&self._data[..len])
    }

    #[inline]
    pub unsafe fn as_slice_mut(&mut self, len: usize) -> &mut [T] {
        MaybeUninitExt::slice_assume_init_mut(&mut self._data[..len])
    }

    #[inline]
    pub unsafe fn clone(&self, len: usize) -> StackVec<T, N>
    where
        T: Clone,
    {
        let mut cloned = StackVec::new();
        cloned.set_len(len);

        for i in 0..len {
            cloned.data_mut()[i] = MaybeUninit::new((&*self._data[i].as_ptr()).clone());
        }

        cloned
    }

    #[inline]
    pub unsafe fn drain<'a, R: RangeBounds<usize>>(
        &'a mut self,
        len: &'a mut usize,
        range: R,
    ) -> OuterLenStackVecDrain<'a, T, N> {
        let len0 = *len;
        let start = match range.start_bound() {
            Bound::Excluded(&start) => start + 1,
            Bound::Included(&start) => start,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Excluded(&end) => end,
            Bound::Included(&end) => end + 1,
            Bound::Unbounded => len0,
        };
        assert!(start <= end && end <= len0);

        *len = start;
        let range_slice = slice::from_raw_parts_mut(self.as_mut_ptr().add(start), end - start);
        OuterLenStackVecDrain {
            tail_start: end,
            tail_len: len0 - end,
            iter: range_slice.iter(),
            vec: ptr::NonNull::from(self),
            vec_len: len,
        }
    }

    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self._data.as_ptr() as *const T
    }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self._data.as_mut_ptr() as *mut T
    }
}

impl<T, const N: usize> Default for OuterLenStackVec<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a, T, const N: usize> OuterLenStackVecDrain<'a, T, N> {
    pub fn as_slice(&self) -> &[T] {
        self.iter.as_slice()
    }
}

impl<'a, T: fmt::Debug, const N: usize> fmt::Debug for OuterLenStackVecDrain<'a, T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("OuterLenStackVecDrain")
            .field(&self.iter.as_slice())
            .finish()
    }
}

impl<'a, T, const N: usize> Iterator for OuterLenStackVecDrain<'a, T, N> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        self.iter.next().map(|el| unsafe { ptr::read(el) })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, T, const N: usize> DoubleEndedIterator for OuterLenStackVecDrain<'a, T, N> {
    #[inline]
    fn next_back(&mut self) -> Option<T> {
        self.iter.next_back().map(|el| unsafe { ptr::read(el) })
    }
}

impl<'a, T, const N: usize> ExactSizeIterator for OuterLenStackVecDrain<'a, T, N> {}

impl<'a, T, const N: usize> Drop for OuterLenStackVecDrain<'a, T, N> {
    fn drop(&mut self) {
        /// Continues dropping the remaining elements in the `Drain`, then moves back the
        /// un-`Drain`ed elements to restore the original `Vec`.
        struct DropGuard<'r, 'a, T, const N: usize>(&'r mut OuterLenStackVecDrain<'a, T, N>);

        impl<'r, 'a, T, const N: usize> Drop for DropGuard<'r, 'a, T, N> {
            fn drop(&mut self) {
                // Continue the same loop we have below. If the loop already finished, this does
                // nothing.
                self.0.for_each(drop);

                if self.0.tail_len > 0 {
                    unsafe {
                        let source_vec = self.0.vec.as_mut();
                        // memmove back untouched tail, update to new length
                        let start = *self.0.vec_len;
                        let tail = self.0.tail_start;
                        if tail != start {
                            let src = source_vec.as_ptr().add(tail);
                            let dst = source_vec.as_mut_ptr().add(start);
                            ptr::copy(src, dst, self.0.tail_len);
                        }
                        *self.0.vec_len = start + self.0.tail_len;
                    }
                }
            }
        }

        // exhaust self first
        while let Some(item) = self.next() {
            let guard = DropGuard(self);
            drop(item);
            mem::forget(guard);
        }

        // Drop a `DropGuard` to move back the non-drained tail of `self`.
        DropGuard(self);
    }
}

#[repr(C)]
pub struct StackVec<T, const N: usize> {
    _data: OuterLenStackVec<T, N>,
    _len: usize,
}

pub struct StackVecIntoIter<T, const N: usize> {
    _data: StackVec<T, N>,
    _start: usize,
}

pub struct StackVecDrain<'a, T, const N: usize> {
    tail_start: usize,
    tail_len: usize,
    vec: ptr::NonNull<StackVec<T, N>>,
    iter: slice::Iter<'a, T>,
}

impl<T, const N: usize> StackVec<T, N> {
    pub fn new() -> Self {
        unsafe { Self::from_raw_parts(OuterLenStackVec::new(), 0) }
    }

    #[inline(always)]
    fn data(&self) -> &[MaybeUninit<T>; N] {
        &self._data._data
    }

    #[inline(always)]
    fn data_mut(&mut self) -> &mut [MaybeUninit<T>; N] {
        &mut self._data._data
    }

    #[inline]
    pub fn as_ptr(&self) -> *const T {
        self._data.as_ptr()
    }

    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut T {
        self._data.as_mut_ptr()
    }

    #[inline(always)]
    pub const fn len(&self) -> usize {
        self._len
    }

    #[inline(always)]
    pub const fn capacity(&self) -> usize {
        N
    }

    #[inline(always)]
    pub const fn is_full(&self) -> bool {
        self.len() == N
    }

    #[inline(always)]
    pub unsafe fn set_len(&mut self, len: usize) {
        self._len = len;
    }

    #[inline(always)]
    pub unsafe fn get_len_mut(&mut self) -> &mut usize {
        &mut self._len
    }

    pub fn into_raw_parts(self) -> (OuterLenStackVec<T, N>, usize) {
        unsafe {
            let mb = ManuallyDrop::new(self);
            (ptr::read(&mb._data), mb._len)
        }
    }

    #[inline(always)]
    pub const unsafe fn from_raw_parts(data: OuterLenStackVec<T, N>, len: usize) -> Self {
        Self {
            _data: data,
            _len: len,
        }
    }

    /// Inserts `item` at `idx` such that `self[idx] == item`, the function returns the overflow.
    #[must_use]
    pub fn insert(&mut self, idx: usize, item: T) -> Option<T> {
        unsafe { self._data.insert(&mut self._len, idx, item) }
    }

    /// Pushes `item` to the end, returns the overflow (the overflow will always be item).
    #[must_use]
    pub fn push(&mut self, item: T) -> Option<T> {
        unsafe { self._data.push(&mut self._len, item) }
    }

    /// Pops the last element.
    pub fn pop(&mut self) -> Option<T> {
        unsafe { self._data.pop(&mut self._len) }
    }

    pub fn split_at(&mut self, left_len: usize) -> Self {
        unsafe { self._data.split_at(&mut self._len, left_len) }
    }

    pub fn remove(&mut self, idx: usize) -> T {
        unsafe { self._data.remove(&mut self._len, idx) }
    }

    pub fn as_slice(&self) -> &[T] {
        unsafe { self._data.as_slice(self.len()) }
    }

    pub fn as_slice_mut(&mut self) -> &mut [T] {
        unsafe { self._data.as_slice_mut(self.len()) }
    }

    pub fn drain<R: RangeBounds<usize>>(&mut self, range: R) -> StackVecDrain<T, N> {
        let len = self.len();
        let start = match range.start_bound() {
            Bound::Excluded(&start) => start + 1,
            Bound::Included(&start) => start,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Excluded(&end) => end,
            Bound::Included(&end) => end + 1,
            Bound::Unbounded => len,
        };
        assert!(start <= end && end <= len);

        unsafe {
            self.set_len(start);
            let range_slice = slice::from_raw_parts_mut(self.as_mut_ptr().add(start), end - start);
            StackVecDrain {
                tail_start: end,
                tail_len: len - end,
                iter: range_slice.iter(),
                vec: ptr::NonNull::from(self),
            }
        }
    }
}

impl<T, const N: usize> core::ops::Deref for StackVec<T, N> {
    type Target = [T];

    #[inline(always)]
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T, const N: usize> core::ops::DerefMut for StackVec<T, N> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_slice_mut()
    }
}

impl<T, const N: usize> Default for StackVec<T, N> {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> Drop for StackVec<T, N> {
    fn drop(&mut self) {
        while let Some(_) = self.pop() {}
    }
}

impl<T: Clone, const N: usize> Clone for StackVec<T, N> {
    fn clone(&self) -> Self {
        unsafe { self._data.clone(self._len) }
    }
}

impl<T: fmt::Debug, const N: usize> fmt::Debug for StackVec<T, N> {
    #[inline(always)]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl<T, const N: usize> IntoIterator for StackVec<T, N> {
    type Item = T;
    type IntoIter = StackVecIntoIter<T, N>;

    fn into_iter(self) -> StackVecIntoIter<T, N> {
        StackVecIntoIter {
            _data: self,
            _start: 0,
        }
    }
}

impl<T, const N: usize> StackVecIntoIter<T, N> {
    #[inline(always)]
    pub fn start(&self) -> usize {
        self._start
    }

    #[inline(always)]
    unsafe fn set_start(&mut self, start: usize) {
        self._start = start;
    }

    #[inline(always)]
    unsafe fn get_start_mut(&mut self) -> &mut usize {
        &mut self._start
    }

    #[inline(always)]
    pub fn end(&self) -> usize {
        self._data.len()
    }

    #[inline(always)]
    unsafe fn set_end(&mut self, end: usize) {
        self._data.set_len(end)
    }

    #[inline(always)]
    unsafe fn get_end_mut(&mut self) -> &mut usize {
        self._data.get_len_mut()
    }
}

impl<T, const N: usize> Iterator for StackVecIntoIter<T, N> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        unsafe {
            if self.start() < self.end() {
                let item = self._data.data()[self.start()].as_ptr().read();
                *self.get_start_mut() += 1;
                Some(item)
            } else {
                None
            }
        }
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let exact = self.len();
        (exact, Some(exact))
    }

    fn count(self) -> usize {
        self.len()
    }
}

impl<T, const N: usize> DoubleEndedIterator for StackVecIntoIter<T, N> {
    #[inline]
    fn next_back(&mut self) -> Option<T> {
        unsafe {
            if self.start() < self.end() {
                *self.get_end_mut() -= 1;
                let item = self._data.data()[self.end()].as_ptr().read();
                Some(item)
            } else {
                None
            }
        }
    }
}

impl<T, const N: usize> ExactSizeIterator for StackVecIntoIter<T, N> {
    #[inline]
    fn len(&self) -> usize {
        self.end() - self.start()
    }
}

impl<T, const N: usize> core::iter::FusedIterator for StackVecIntoIter<T, N> {}

impl<T: Clone, const N: usize> Clone for StackVecIntoIter<T, N> {
    fn clone(&self) -> Self {
        unsafe {
            let mut clone = StackVec::new().into_iter();
            clone.set_start(self.start());
            clone.set_end(self.end());

            for i in self._start..self._data.len() {
                clone._data[i] = self._data[i].clone();
            }
            clone
        }
    }
}

impl<T: fmt::Debug, const N: usize> fmt::Debug for StackVecIntoIter<T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("StackVecIntoIter")
            .field(&&self._data[self._start..])
            .finish()
    }
}

impl<T, const N: usize> Drop for StackVecIntoIter<T, N> {
    fn drop(&mut self) {
        // self.for_each()
        while let Some(_) = self.next() {}

        unsafe {
            self._data.set_len(0);
        }
    }
}

impl<'a, T, const N: usize> StackVecDrain<'a, T, N> {
    pub fn as_slice(&self) -> &[T] {
        self.iter.as_slice()
    }
}

impl<'a, T: fmt::Debug, const N: usize> fmt::Debug for StackVecDrain<'a, T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("StackVecDrain")
            .field(&self.iter.as_slice())
            .finish()
    }
}

impl<'a, T, const N: usize> Iterator for StackVecDrain<'a, T, N> {
    type Item = T;

    #[inline]
    fn next(&mut self) -> Option<T> {
        self.iter.next().map(|el| unsafe { ptr::read(el) })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl<'a, T, const N: usize> DoubleEndedIterator for StackVecDrain<'a, T, N> {
    #[inline]
    fn next_back(&mut self) -> Option<T> {
        self.iter.next_back().map(|el| unsafe { ptr::read(el) })
    }
}

impl<'a, T, const N: usize> ExactSizeIterator for StackVecDrain<'a, T, N> {}

impl<'a, T, const N: usize> Drop for StackVecDrain<'a, T, N> {
    fn drop(&mut self) {
        /// Continues dropping the remaining elements in the `Drain`, then moves back the
        /// un-`Drain`ed elements to restore the original `Vec`.
        struct DropGuard<'r, 'a, T, const N: usize>(&'r mut StackVecDrain<'a, T, N>);

        impl<'r, 'a, T, const N: usize> Drop for DropGuard<'r, 'a, T, N> {
            fn drop(&mut self) {
                // Continue the same loop we have below. If the loop already finished, this does
                // nothing.
                self.0.for_each(drop);

                if self.0.tail_len > 0 {
                    unsafe {
                        let source_vec = self.0.vec.as_mut();
                        // memmove back untouched tail, update to new length
                        let start = source_vec.len();
                        let tail = self.0.tail_start;
                        if tail != start {
                            let src = source_vec.as_ptr().add(tail);
                            let dst = source_vec.as_mut_ptr().add(start);
                            ptr::copy(src, dst, self.0.tail_len);
                        }
                        source_vec.set_len(start + self.0.tail_len);
                    }
                }
            }
        }

        // exhaust self first
        while let Some(item) = self.next() {
            let guard = DropGuard(self);
            drop(item);
            mem::forget(guard);
        }

        // Drop a `DropGuard` to move back the non-drained tail of `self`.
        DropGuard(self);
    }
}
