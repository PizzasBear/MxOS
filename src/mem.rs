//! This module contains a lot of the structures and algorithms related to memory allocation.
//!

use core::marker::PhantomData;
use core::mem::size_of;
use core::ops::Range;
use multiboot2::{BootInformation, MemoryArea, MemoryMapTag};
use x86_64::structures::paging::{
    FrameAllocator, PageSize, PageTable, PageTableFlags, PhysFrame, Size4KiB,
};
use x86_64::{PhysAddr, VirtAddr};

/// The page allocator trait.
pub trait PageAllocator<S: PageSize> {
    /// Allocates a page and returns it's virtual address.
    fn allocate_page(&mut self) -> VirtAddr;
    /// Allocates multiple pages continuously.
    fn allocate_pages(&mut self, num: u64) -> Option<VirtAddr>;
}

/// The page deallocator.
pub trait PageDeallocator<S: PageSize> {
    /// The error type.
    type Err;

    /// Deallocates a page and returns it's virtual address.
    fn deallocate_page(&mut self) -> Result<(), Self::Err>;
    /// Deallocates multiple pages continuously.
    fn deallocate_pages(&mut self, num: u64) -> Result<(), Self::Err>;
}

/// A very simple frame allocator, it can't deallocate any frames.
/// It will be used for setup of the main frame allocator.
#[derive(Debug)]
pub struct BasicFrameAllocator<'a> {
    current_frame: u64,
    taken_areas: [Range<u64>; 2],
    current_area: Option<&'a MemoryArea>,
    memory_area_index: usize,
    memory_map_tag: &'a MemoryMapTag,
}

impl<'a> BasicFrameAllocator<'a> {
    /// Create a new BasicFrameAllocator. Taken areas are addresses that are taken by either the
    /// kernel or the Multiboot2 information structure.
    pub fn new(taken_areas: [Range<u64>; 2], memory_map_tag: &'a MemoryMapTag) -> Self {
        Self {
            current_frame: 4096,
            current_area: memory_map_tag.memory_areas().next(),
            memory_area_index: 0,
            memory_map_tag,
            taken_areas,
        }
    }
}

impl<'a> PageAllocator<Size4KiB> for BasicFrameAllocator<'a> {
    fn allocate_page(&mut self) -> VirtAddr {
        unsafe {
            use x86_64::registers::control::Cr3;

            let mut frames = [PhysFrame::from_start_address(PhysAddr::new(0)).unwrap(); 16];
            frames[0] = self.allocate_frame().unwrap();
            let alloc_page_addr = VirtAddr::new(frames[0].start_address().as_u64());

            let mut frames_start = 0;
            let mut frames_len = 1;
            // let virt_addr = VirtAddr::new(frames[0].start_address().as_u64());

            let (level_4_page_frame, _) = Cr3::read();
            let level_4_page =
                &mut *(level_4_page_frame.start_address().as_u64() as *mut PageTable);

            // Allocate frames for the pages
            while 0 < frames_len {
                let addr = frames[frames_start].start_address().as_u64();
                let virt_addr = VirtAddr::new(addr);

                frames_start = (frames_start + 1) % frames.len();
                frames_len -= 1;

                let p4_entry = &mut level_4_page[virt_addr.p4_index()];
                if p4_entry.is_unused() {
                    let frame = self.allocate_frame().unwrap();
                    frames[(frames_start + frames_len) % frames.len()] = frame;
                    frames_len += 1;
                    assert!(frames_len <= frames.len(), "Too many frames");

                    p4_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                    let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
                    p3.zero();
                }
                let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);

                let p3_entry = &mut p3[virt_addr.p3_index()];
                if p3_entry.is_unused() {
                    let frame = self.allocate_frame().unwrap();
                    frames[(frames_start + frames_len) % frames.len()] = frame;
                    frames_len += 1;
                    assert!(frames_len <= frames.len(), "Too many frames");

                    p3_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                    let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
                    p2.zero();
                }
                let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);

                let p2_entry = &mut p2[virt_addr.p2_index()];
                if p2_entry.is_unused() {
                    let frame = self.allocate_frame().unwrap();
                    frames[(frames_start + frames_len) % frames.len()] = frame;
                    frames_len += 1;
                    assert!(frames_len <= frames.len(), "Too many frames");

                    p2_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                    let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
                    p1.zero();
                }
                let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);

                let p1_entry = &mut p1[virt_addr.p1_index()];
                assert!(p1_entry.is_unused());
                p1_entry.set_frame(
                    PhysFrame::from_start_address(PhysAddr::new(addr)).unwrap(),
                    PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
                );
            }
            alloc_page_addr
        }
    }

    fn allocate_pages(&mut self, num: u64) -> Option<VirtAddr> {
        let first = self.allocate_page();
        for i in 1..num {
            let page = self.allocate_page();
            assert_eq!(first.as_u64() + 4096 * i, page.as_u64());
        }
        Some(first)
    }
}

unsafe impl<'a> FrameAllocator<Size4KiB> for BasicFrameAllocator<'a> {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        let current_area = self.current_area?;

        if self.current_frame < current_area.start_address() {
            self.current_frame = current_area.start_address() + 4095 & !4095;
        }

        if current_area.end_address() < self.current_frame + 4096 {
            self.memory_area_index += 1;
            self.current_area = self
                .memory_map_tag
                .memory_areas()
                .nth(self.memory_area_index);
            return self.allocate_frame();
        }
        for area in &self.taken_areas {
            if area.start < self.current_frame + 4096 && self.current_frame < area.end {
                self.current_frame = area.end + 4095 & !4095;
                return self.allocate_frame();
            }
        }
        self.current_frame += 4096;

        Some(PhysFrame::from_start_address(PhysAddr::new(self.current_frame - 4096)).unwrap())
    }
}

/// A slab allocator, that allocates only type T. It needs a page allocator, but it never
/// deallocates.
pub struct SlabAllocator<T> {
    free_size: u64,
    free_list: *mut FreeList,
    _phantom: PhantomData<T>,
}

impl<T: Sized> SlabAllocator<T> {
    /// Creates a new slab allocator from a page allocator.
    pub fn new<A: PageAllocator<Size4KiB>>(allocator: &mut A) -> Self {
        unsafe {
            const INIT_FREE_SIZE: u64 = 16 * 4096;

            assert_eq!(size_of::<FreeList>(), 16);
            assert!(16 < size_of::<T>() && size_of::<T>() < 4096);
            assert_eq!(size_of::<T>() & 0xf, 0);

            Self {
                free_size: INIT_FREE_SIZE,
                free_list: {
                    let pages = allocator
                        .allocate_pages(INIT_FREE_SIZE / 4096)
                        .unwrap()
                        .as_mut_ptr();
                    *pages = FreeList {
                        size: INIT_FREE_SIZE,
                        next: None,
                    };
                    pages
                },
                _phantom: PhantomData,
            }
        }
    }

    /// Allocates a pointer to `T`.
    pub unsafe fn malloc<A: PageAllocator<Size4KiB>>(&mut self, allocator: &mut A) -> *mut T {
        const MIN_FREE_SIZE: u64 = 8 * 4096;

        if self.free_size < MIN_FREE_SIZE {
            self.free_size += MIN_FREE_SIZE;
            let pages = allocator
                .allocate_pages(MIN_FREE_SIZE / 4096)
                .unwrap()
                .as_mut_ptr();
            *pages = FreeList {
                size: MIN_FREE_SIZE,
                next: Some(self.free_list),
            };
            self.free_list = pages;
        }

        let FreeList { size, next } = *self.free_list;
        if (size_of::<T>() as u64) < size {
            let ptr = self.free_list as *mut T;
            self.free_list = self.free_list.add(size_of::<T>() / 16);
            *self.free_list = FreeList {
                size: size - size_of::<T>() as u64,
                next,
            };
            self.free_size -= size_of::<T>() as u64;

            ptr
        } else if size_of::<T>() == size as usize {
            let ptr = self.free_list as *mut T;
            self.free_list = next.unwrap();
            self.free_size -= size_of::<T>() as u64;

            ptr
        } else {
            self.free_list = next.unwrap();
            self.free_size -= size;

            self.malloc(allocator)
        }
    }

    /// Deallocates a pointer to `T`;
    pub unsafe fn free(&mut self, ptr: *mut T) {
        let free_list = self.free_list;
        self.free_list = ptr as *mut FreeList;
        *self.free_list = FreeList {
            size: size_of::<T>() as _,
            next: Some(free_list),
        };
        self.free_size += size_of::<T>() as u64;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(16))]
struct FreeList {
    size: u64,
    next: Option<*mut FreeList>,
}

/// This function creates a new page table that contains the kernel and the multiboot information.
pub unsafe fn reset_page_table<FA: FrameAllocator<Size4KiB>>(
    kernel_start: u64,
    kernel_end: u64,
    boot_info: &BootInformation,
    frame_allocator: &mut FA,
) {
    // use core::ptr;
    use x86_64::registers::control::Cr3;

    let mut frames = [PhysFrame::from_start_address(PhysAddr::new(0)).unwrap(); 16];
    frames[0] = frame_allocator.allocate_frame().unwrap();
    let mut frames_start = 0;
    let mut frames_len = 1;
    // let virt_addr = VirtAddr::new(frames[0].start_address().as_u64());

    let new_level_4_page_frame = frames[0];
    let new_level_4_page = &mut *(frames[0].start_address().as_u64() as *mut PageTable);
    new_level_4_page.zero();

    for addr in ((kernel_start & !4095)..(kernel_end + 4095 & !4095))
        .step_by(4096)
        .chain(
            ((boot_info.start_address() as u64 & !4095)
                ..(boot_info.end_address() as u64 + 4095 & !4095))
                .step_by(4096),
        )
    {
        let virt_addr = VirtAddr::new(addr);

        let p4_entry = &mut new_level_4_page[virt_addr.p4_index()];
        if p4_entry.is_unused() {
            let frame = frame_allocator.allocate_frame().unwrap();
            frames[(frames_start + frames_len) % frames.len()] = frame;
            frames_len += 1;
            assert!(frames_len <= frames.len(), "Too many frames");

            p4_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
            p3.zero();
        }
        let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);

        let p3_entry = &mut p3[virt_addr.p3_index()];
        if p3_entry.is_unused() {
            let frame = frame_allocator.allocate_frame().unwrap();
            frames[(frames_start + frames_len) % frames.len()] = frame;
            frames_len += 1;
            assert!(frames_len <= frames.len(), "Too many frames");

            p3_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
            p2.zero();
        }
        let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);

        let p2_entry = &mut p2[virt_addr.p2_index()];
        if p2_entry.is_unused() {
            let frame = frame_allocator.allocate_frame().unwrap();
            frames[(frames_start + frames_len) % frames.len()] = frame;
            frames_len += 1;
            assert!(frames_len <= frames.len(), "Too many frames");

            p2_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
            let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
            p1.zero();
        }
        let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);

        let p1_entry = &mut p1[virt_addr.p1_index()];
        assert!(p1_entry.is_unused());
        p1_entry.set_frame(
            PhysFrame::from_start_address(PhysAddr::new(addr)).unwrap(),
            PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
        );

        // Allocate frames for the pages
        while 0 < frames_len {
            let addr = frames[frames_start].start_address().as_u64();
            let virt_addr = VirtAddr::new(addr);

            frames_start = (frames_start + 1) % frames.len();
            frames_len -= 1;

            let p4_entry = &mut new_level_4_page[virt_addr.p4_index()];
            if p4_entry.is_unused() {
                let frame = frame_allocator.allocate_frame().unwrap();
                frames[(frames_start + frames_len) % frames.len()] = frame;
                frames_len += 1;
                assert!(frames_len <= frames.len(), "Too many frames");

                p4_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
                p3.zero();
            }
            let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);

            let p3_entry = &mut p3[virt_addr.p3_index()];
            if p3_entry.is_unused() {
                let frame = frame_allocator.allocate_frame().unwrap();
                frames[(frames_start + frames_len) % frames.len()] = frame;
                frames_len += 1;
                assert!(frames_len <= frames.len(), "Too many frames");

                p3_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
                p2.zero();
            }
            let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);

            let p2_entry = &mut p2[virt_addr.p2_index()];
            if p2_entry.is_unused() {
                let frame = frame_allocator.allocate_frame().unwrap();
                frames[(frames_start + frames_len) % frames.len()] = frame;
                frames_len += 1;
                assert!(frames_len <= frames.len(), "Too many frames");

                p2_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
                let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
                p1.zero();
            }
            let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);

            let p1_entry = &mut p1[virt_addr.p1_index()];
            assert!(p1_entry.is_unused());
            p1_entry.set_frame(
                PhysFrame::from_start_address(PhysAddr::new(addr)).unwrap(),
                PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
            );
        }
    }

    log::info!("Resetting the page table...");
    let (_, cr3_flags) = Cr3::read();
    Cr3::write(new_level_4_page_frame, cr3_flags);
    log::info!("The page table is reset");
}
