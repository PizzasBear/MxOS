use crate::stack_vec::StackVec;
use core::marker::PhantomData;
use core::mem;

// pub struct RefMutStack<'a, T>(Vec<*mut T>, PhantomData<&'a mut T>);
//
// impl<'a, T> RefMutStack<'a, T> {
//     #[inline]
//     pub fn new() -> Self {
//         Self(Vec::new(), PhantomData)
//     }
//
//     #[inline]
//     pub fn with_root(root: &'a mut T) -> Self {
//         Self(vec![root], PhantomData)
//     }
//
//     #[inline]
//     pub fn with_capacity(capacity: usize) -> Self {
//         Self(Vec::with_capacity(capacity), PhantomData)
//     }
//
//     #[inline]
//     pub fn reserve(&mut self, additional: usize) {
//         self.0.reserve(additional);
//     }
//
//     #[inline]
//     pub fn is_empty(&self) -> bool {
//         self.0.is_empty()
//     }
//
//     #[inline]
//     pub fn len(&self) -> usize {
//         self.0.len()
//     }
//
//     #[inline]
//     pub fn capacity(&self) -> usize {
//         self.0.capacity()
//     }
//
//     pub fn push_root(&mut self, root: &'a mut T) -> Self {
//         if self.0.is_empty() {
//             self.0.push(root);
//             Self::new()
//         } else {
//             mem::replace(self, Self::with_root(root))
//         }
//     }
//
//     #[inline]
//     pub fn peek(&self) -> Option<&T> {
//         unsafe { Some(&**self.0.last()?) }
//     }
//
//     #[inline]
//     pub fn peek_mut(&mut self) -> Option<&mut T> {
//         unsafe { Some(&mut **self.0.last_mut()?) }
//     }
//
//     #[inline]
//     pub fn push<F: FnOnce(&'a mut T) -> &'a mut T>(&mut self, f: F) -> bool {
//         unsafe {
//             let x = match self.0.last_mut() {
//                 Some(x) => &mut *(*x as *mut T),
//                 None => return false,
//             };
//             self.0.push(f(x));
//             true
//         }
//     }
//
//     // #[inline]
//     // pub fn try_push<F: FnOnce(&'a mut T) -> Option<&'a mut T>>(&mut self, f: F) -> bool {
//     //     unsafe {
//     //         let x = match self.0.last_mut() {
//     //             Some(x) => &mut *(*x as *mut T),
//     //             None => return false,
//     //         };
//     //         self.0.push(match f(x) {
//     //             Some(x) => x,
//     //             None => return false,
//     //         });
//     //         true
//     //     }
//     // }
//
//     #[inline]
//     pub fn try_push<'b, E, F>(&'b mut self, f: F) -> Result<bool, E>
//     where
//         E: 'b,
//         F: FnOnce(&'a mut T) -> Result<&'a mut T, E>,
//     {
//         unsafe {
//             let x = match self.0.last_mut() {
//                 Some(x) => &mut *(*x as *mut T),
//                 None => return Ok(false),
//             };
//             self.0.push(match f(x) {
//                 Ok(x) => x,
//                 Err(e) => return Err(e),
//             });
//             Ok(true)
//         }
//     }
//
//     #[inline]
//     pub fn pop(self: &mut Self) -> Option<&'a mut T> {
//         let popped = self.0.pop();
//         if self.is_empty() {
//             popped.map(|x| unsafe { &mut *x })
//         } else {
//             None
//         }
//     }
// }

pub struct OnStackRefMutStack<'a, T, const N: usize>(StackVec<*mut T, N>, PhantomData<&'a mut T>);

impl<'a, T, const N: usize> OnStackRefMutStack<'a, T, N> {
    #[inline]
    pub fn new() -> Self {
        Self(StackVec::new(), PhantomData)
    }

    #[inline]
    pub fn with_root(root: &'a mut T) -> Self {
        let mut vec = StackVec::<*mut T, N>::new();
        assert!(vec.push(root).is_none());
        Self(vec, PhantomData)
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[inline]
    pub fn capacity(&self) -> usize {
        self.0.capacity()
    }

    pub fn push_root(&mut self, root: &'a mut T) -> Self {
        if self.0.is_empty() {
            assert!(self.0.push(root).is_none());
            Self::new()
        } else {
            mem::replace(self, Self::with_root(root))
        }
    }

    #[inline]
    pub fn peek(&self) -> Option<&T> {
        unsafe { Some(&**self.0.last()?) }
    }

    #[inline]
    pub fn peek_mut(&mut self) -> Option<&mut T> {
        unsafe { Some(&mut **self.0.last_mut()?) }
    }

    #[inline]
    pub fn push<F: FnOnce(&'a mut T) -> &'a mut T>(&mut self, f: F) -> bool {
        if self.0.is_full() {
            false
        } else {
            unsafe {
                let x = match self.0.last_mut() {
                    Some(x) => &mut *(*x as *mut T),
                    None => return false,
                };
                assert!(self.0.push(f(x)).is_none());
                true
            }
        }
    }

    // #[inline]
    // pub fn try_push<F: FnOnce(&'a mut T) -> Option<&'a mut T>>(&mut self, f: F) -> bool {
    //     unsafe {
    //         let x = match self.0.last_mut() {
    //             Some(x) => &mut *(*x as *mut T),
    //             None => return false,
    //         };
    //         self.0.push(match f(x) {
    //             Some(x) => x,
    //             None => return false,
    //         });
    //         true
    //     }
    // }

    /// Trys to push
    #[inline]
    pub fn try_push<'b, E, F>(&'b mut self, f: F) -> Result<bool, E>
    where
        E: 'b,
        F: FnOnce(&'a mut T) -> Result<&'a mut T, E>,
    {
        if self.0.is_full() {
            Ok(false)
        } else {
            unsafe {
                let x = match self.0.last_mut() {
                    Some(x) => &mut **x,
                    None => return Ok(false),
                };
                assert!(self
                    .0
                    .push(match f(x) {
                        Ok(x) => x,
                        Err(e) => return Err(e),
                    })
                    .is_none());
                Ok(true)
            }
        }
    }

    /// `self.pop()` returns `Some` only if the root was popped, otherwise it returns `None`
    #[inline]
    pub fn pop(self: &mut Self) -> Option<&'a mut T> {
        let popped = self.0.pop();
        if self.is_empty() {
            popped.map(|x| unsafe { &mut *x })
        } else {
            None
        }
    }
}

// #[test]
// fn ref_stack() {
//     struct SelfRef {
//         num: u64,
//         ptr: Option<Box<Self>>,
//     }
//
//     let mut self_ref = SelfRef {
//         num: 8,
//         ptr: Some(Box::new(SelfRef {
//             num: 5,
//             ptr: Some(Box::new(SelfRef {
//                 num: 2,
//                 ptr: Some(Box::new(SelfRef { num: 7, ptr: None })),
//             })),
//         })),
//     };
//     let mut stack = RefStack::with_root(&mut self_ref);
//
//     let root = stack.peek_mut().unwrap();
//     root.num += 1;
//     println!("{}", root.num);
//
//     drop(stack);
// }
