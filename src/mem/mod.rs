//! This module contains a lot of the structures and algorithms related to memory allocation.
//!

mod btree;
mod bump;
mod slab;

pub use slab::{SlabAllocator, SlabBox};

use btree::BTree;
use bump::BumpAllocator;

use core::mem::MaybeUninit;
use core::ptr;
use core::slice;
use multiboot2::{BootInformation, MemoryMapTag};
use x86_64::structures::paging::{FrameAllocator, PageSize, PageTable, Size2MiB};

// /// The page allocator trait.
// pub trait PageAllocator<S: PageSize> {
//     /// Allocates a page and returns it's virtual address.
//     fn allocate_page(&mut self) -> VirtAddr;
//     /// Allocates multiple pages continuously.
//     fn allocate_pages(&mut self, num: u64) -> Option<VirtAddr>;
// }

// /// The page deallocator.
// pub trait PageDeallocator<S: PageSize> {
//     /// The error type.
//     type Err;
//
//     /// Deallocates a page and returns it's virtual address.
//     fn deallocate_page(&mut self) -> Result<(), Self::Err>;
//     /// Deallocates multiple pages continuously.
//     fn deallocate_pages(&mut self, num: u64) -> Result<(), Self::Err>;
// }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C, align(16))]
struct BuddyFreeList {
    ptr: usize,
    next: Option<ptr::NonNull<BuddyFreeList>>,
}

struct Buddies {
    bitmap: &'static mut [u64],
    free_list: Option<ptr::NonNull<BuddyFreeList>>,
    num_buddies: usize,
}

struct BuddyAllocator<const N: usize> {
    buddies: [Buddies; N],
    free_list_alloc: SlabAllocator<BuddyFreeList>,
    base_size: usize,
    offset: usize,
}

impl<const N: usize> BuddyAllocator<N> {
    pub fn malloc(&mut self, order: usize) -> Option<usize> {
        unsafe {
            let order_buddy_size = self.base_size << order;

            while let Some(free_list) = self.buddies[order].free_list {
                let BuddyFreeList { ptr, next } = *free_list.as_ref();
                self.buddies[order].free_list = next;

                self.free_list_alloc.free(free_list);

                let chunk_ptr = ptr / (self.base_size << order);
                if self.buddies[order].bitmap[chunk_ptr >> 6] & 1 << (chunk_ptr & 63) != 0 {
                    continue;
                }

                self.buddies[order].bitmap[chunk_ptr >> 6] ^= 1 << (chunk_ptr & 63);
                return Some(self.offset + ptr);
            }

            if order == self.buddies.len() - 1 {
                None
            } else {
                let ptr = self.malloc(order + 1)?;

                let chunk_ptr = (ptr - self.offset) / order_buddy_size;
                self.buddies[order].bitmap[chunk_ptr >> 6] ^= 1 << (chunk_ptr + 1 & 63);

                Some(ptr)
            }
        }
    }

    pub fn free(&mut self, ptr: usize, order: usize) {
        unsafe {
            let chunk_ptr = (ptr - self.offset) / (self.base_size << order);
            if order < self.buddies.len() - 1
                && self.buddies[order].bitmap[chunk_ptr >> 6] & 1 << (chunk_ptr & 63 ^ 1) == 0
            {
                self.buddies[order].bitmap[chunk_ptr >> 6] ^= 1 << (chunk_ptr & 63 ^ 1);

                self.free(ptr, order + 1);
            } else {
                self.buddies[order].bitmap[chunk_ptr >> 6] ^= 1 << (chunk_ptr & 63);

                let mut free_list = self
                    .free_list_alloc
                    .malloc()
                    .expect("Failed to allocate new buddy free list");
                *free_list.as_mut() = BuddyFreeList {
                    ptr: ptr - self.offset,
                    next: self.buddies[order].free_list,
                };
                self.buddies[order].free_list = Some(free_list);
            }
        }
    }

    pub unsafe fn mark_as_used(&mut self, start_address: usize, end_address: usize) {
        let mut start_address = (start_address - self.offset) / self.base_size;
        let mut end_address = (end_address - 1 - self.offset + self.base_size - 1) / self.base_size;

        for order in 0..self.buddies.len() - 1 {
            if end_address < start_address {
                return;
            }
            if start_address ^ end_address == 1 {
                self.buddies[order + 1].bitmap[start_address >> 7] |= 1 << (start_address / 2 & 63);
                return;
            }

            let bitmap = &mut self.buddies[order].bitmap;
            if start_address & 1 == 1
                && bitmap[start_address >> 6] & 1 << (start_address - 1 & 63) != 0
                && bitmap[start_address >> 6] & 1 << (start_address & 63) != 0
            {
                let mut ptr = start_address / 2;
                let mut order = order + 1;
                loop {
                    let bitmap = &mut self.buddies[order].bitmap;
                    if bitmap[ptr >> 6] & 1 << (ptr & 63) != 0 {
                        if ptr & 1 == 1 {
                            let mut free_list = self.free_list_alloc.malloc().unwrap();
                            *free_list.as_mut() = BuddyFreeList {
                                ptr: self.offset + self.base_size * (ptr - 1 << order),
                                next: self.buddies[order].free_list,
                            };
                            self.buddies[order].free_list = Some(free_list);
                            // flips 1 to 0
                            bitmap[ptr >> 6] ^= 1 << (ptr - 1 & 63);
                        }

                        ptr /= 2;
                        order += 1;
                    } else {
                        // flips 0 to 1
                        bitmap[ptr >> 6] ^= 1 << (ptr & 63);
                        break;
                    }
                }
            }

            let bitmap = &mut self.buddies[order].bitmap;
            if end_address & 1 == 1
                && bitmap[end_address >> 6] & 1 << (end_address - 1 & 63) != 0
                && bitmap[end_address >> 6] & 1 << (end_address & 63) != 0
            {
                let mut ptr = end_address / 2;
                let mut order = order + 1;
                loop {
                    let bitmap = &mut self.buddies[order].bitmap;
                    if bitmap[ptr >> 6] & 1 << (ptr & 63) != 0 {
                        if ptr & 1 == 0 {
                            let mut free_list = self.free_list_alloc.malloc().unwrap();
                            *free_list.as_mut() = BuddyFreeList {
                                ptr: self.offset + self.base_size * (ptr + 1 << order),
                                next: self.buddies[order].free_list,
                            };
                            self.buddies[order].free_list = Some(free_list);
                            // flips 1 to 0
                            bitmap[ptr >> 6] ^= 1 << (ptr + 1 & 63);
                        }

                        ptr /= 2;
                        order += 1;
                    } else {
                        // flips 0 to 1
                        bitmap[ptr >> 6] ^= 1 << (ptr & 63);
                        break;
                    }
                }
            }

            let bitmap = &mut self.buddies[order].bitmap;

            bitmap[start_address >> 6] |= 1 << (start_address & 63);
            bitmap[end_address >> 6] |= 1 << (end_address & 63);

            start_address = (start_address + 1) / 2;
            end_address = (end_address - 1) / 2;
        }

        if end_address < start_address {
            return;
        }

        let bitmap = &mut self.buddies[self.buddies.len() - 1].bitmap;
        end_address += 1;
        if start_address >> 6 == end_address >> 6 {
            bitmap[end_address >> 6] |= (1 << (end_address & 63)) - (1 << (start_address & 63));
        } else {
            bitmap[start_address >> 6] |= !((1 << (start_address & 63)) - 1);
            bitmap[end_address >> 6] |= (1 << (end_address & 63)) - 1;
            for i in (start_address >> 6) + 1..end_address >> 6 {
                bitmap[i] |= !0;
            }
        }
    }
}

const GLOBAL_BUDDY_DEPTH: usize = 8;

/// The global binary buddy memory allocator
pub struct GlobalBuddyAllocator {
    buddy_alloc: BuddyAllocator<GLOBAL_BUDDY_DEPTH>,
    virt_addr_alloc: BTree<usize, u64>,
    page_table_alloc: SlabAllocator<PageTable>,
}

impl GlobalBuddyAllocator {
    /// Creates a new GlobalBuddyAllocator
    pub unsafe fn new(
        kernel_start: usize,
        kernel_end: usize,
        boot_info: &BootInformation,
        memory_map_tag: &MemoryMapTag,
    ) -> Self {
        let mem_size = memory_map_tag
            .memory_areas()
            .map(|area| area.end_address())
            .max()
            .unwrap();
        const TOP_BLOCK_SIZE: usize = 1 << 20 + GLOBAL_BUDDY_DEPTH;

        let mem_size = mem_size as usize & !(TOP_BLOCK_SIZE - 1);
        assert!(mem_size as u64 / Size2MiB::SIZE / 8 <= Size2MiB::SIZE / 2);

        let mut bump_allocator = BumpAllocator::new(
            [
                kernel_start..kernel_end,
                boot_info.start_address()..boot_info.end_address(),
            ],
            memory_map_tag,
        );

        let buddies_frame = bump_allocator
            .allocate_frame()
            .expect("Couldn't allocate a frame for the buddies");
        let free_list_alloc_frame = bump_allocator
            .allocate_frame()
            .expect("Couldn't allocate a frame for the buddies' free list slab allocator");

        let free_list_alloc = SlabAllocator::new(slice::from_raw_parts_mut(
            free_list_alloc_frame.start_address().as_u64() as _,
            Size2MiB::SIZE as _,
        ));

        let mut buddy_alloc = BuddyAllocator::<GLOBAL_BUDDY_DEPTH> {
            buddies: MaybeUninit::uninit().assume_init(),
            free_list_alloc,
            base_size: 0x200000,
            offset: 0,
        };

        let buddies_addr = buddies_frame.start_address().as_u64() as *mut u64;
        for (i, buddies) in buddy_alloc.buddies.iter_mut().enumerate() {
            let num_buddies = mem_size as usize >> 21 + i;
            *buddies = Buddies {
                num_buddies,
                bitmap: slice::from_raw_parts_mut(
                    buddies_addr.add(0x200000 - (0x200000 >> i)),
                    (num_buddies + 63) / 64,
                ),
                free_list: None,
            };
            if i == GLOBAL_BUDDY_DEPTH - 1 {
                buddies.bitmap.fill(0);
            } else {
                buddies.bitmap.fill(!0);
            }
        }

        let top_buddies = &mut buddy_alloc.buddies[GLOBAL_BUDDY_DEPTH - 1];
        for i in (0..top_buddies.bitmap.len() * 8).rev() {
            let mut free_list = buddy_alloc
                .free_list_alloc
                .malloc()
                .expect("Failed to allocate a free list");

            *free_list.as_mut() = BuddyFreeList {
                ptr: (TOP_BLOCK_SIZE * i) as _,
                next: top_buddies.free_list,
            };
            top_buddies.free_list = Some(free_list);
        }
        buddy_alloc.mark_as_used(kernel_start, kernel_end);
        buddy_alloc.mark_as_used(boot_info.start_address(), boot_info.end_address());
        buddy_alloc.mark_as_used(
            buddies_frame.start_address().as_u64() as _,
            (buddies_frame.start_address().as_u64() + Size2MiB::SIZE) as _,
        );
        buddy_alloc.mark_as_used(
            free_list_alloc_frame.start_address().as_u64() as _,
            (free_list_alloc_frame.start_address().as_u64() + Size2MiB::SIZE) as _,
        );
        buddy_alloc.mark_as_used(0, 0x200000);

        let page_table_alloc = SlabAllocator::new(slice::from_raw_parts_mut(
            buddy_alloc.malloc(0).unwrap() as _,
            buddy_alloc.base_size,
        ));

        Self {
            buddy_alloc,
            page_table_alloc,
        }
    }
}

// /// This function creates a new page table that contains the kernel and the multiboot information.
// pub unsafe fn reset_page_table<FA: FrameAllocator<Size4KiB>>(
//     kernel_start: u64,
//     kernel_end: u64,
//     boot_info: &BootInformation,
//     frame_allocator: &mut FA,
// ) {
//     // use core::ptr;
//     use x86_64::registers::control::Cr3;
//
//     let mut frames = [PhysFrame::from_start_address(PhysAddr::new(0)).unwrap(); 16];
//     frames[0] = frame_allocator.allocate_frame().unwrap();
//     let mut frames_start = 0;
//     let mut frames_len = 1;
//     // let virt_addr = VirtAddr::new(frames[0].start_address().as_u64());
//
//     let new_level_4_page_frame = frames[0];
//     let new_level_4_page = &mut *(frames[0].start_address().as_u64() as *mut PageTable);
//     new_level_4_page.zero();
//
//     for addr in ((kernel_start & !4095)..(kernel_end + 4095 & !4095))
//         .step_by(4096)
//         .chain(
//             ((boot_info.start_address() as u64 & !4095)
//                 ..(boot_info.end_address() as u64 + 4095 & !4095))
//                 .step_by(4096),
//         )
//     {
//         let virt_addr = VirtAddr::new(addr);
//
//         let p4_entry = &mut new_level_4_page[virt_addr.p4_index()];
//         if p4_entry.is_unused() {
//             let frame = frame_allocator.allocate_frame().unwrap();
//             frames[(frames_start + frames_len) % frames.len()] = frame;
//             frames_len += 1;
//             assert!(frames_len <= frames.len(), "Too many frames");
//
//             p4_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
//             let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
//             p3.zero();
//         }
//         let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
//
//         let p3_entry = &mut p3[virt_addr.p3_index()];
//         if p3_entry.is_unused() {
//             let frame = frame_allocator.allocate_frame().unwrap();
//             frames[(frames_start + frames_len) % frames.len()] = frame;
//             frames_len += 1;
//             assert!(frames_len <= frames.len(), "Too many frames");
//
//             p3_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
//             let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
//             p2.zero();
//         }
//         let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
//
//         let p2_entry = &mut p2[virt_addr.p2_index()];
//         if p2_entry.is_unused() {
//             let frame = frame_allocator.allocate_frame().unwrap();
//             frames[(frames_start + frames_len) % frames.len()] = frame;
//             frames_len += 1;
//             assert!(frames_len <= frames.len(), "Too many frames");
//
//             p2_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
//             let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
//             p1.zero();
//         }
//         let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
//
//         let p1_entry = &mut p1[virt_addr.p1_index()];
//         assert!(p1_entry.is_unused());
//         p1_entry.set_frame(
//             PhysFrame::from_start_address(PhysAddr::new(addr)).unwrap(),
//             PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
//         );
//
//         // Allocate frames for the pages
//         while 0 < frames_len {
//             let addr = frames[frames_start].start_address().as_u64();
//             let virt_addr = VirtAddr::new(addr);
//
//             frames_start = (frames_start + 1) % frames.len();
//             frames_len -= 1;
//
//             let p4_entry = &mut new_level_4_page[virt_addr.p4_index()];
//             if p4_entry.is_unused() {
//                 let frame = frame_allocator.allocate_frame().unwrap();
//                 frames[(frames_start + frames_len) % frames.len()] = frame;
//                 frames_len += 1;
//                 assert!(frames_len <= frames.len(), "Too many frames");
//
//                 p4_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
//                 let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
//                 p3.zero();
//             }
//             let p3 = &mut *(p4_entry.addr().as_u64() as *mut PageTable);
//
//             let p3_entry = &mut p3[virt_addr.p3_index()];
//             if p3_entry.is_unused() {
//                 let frame = frame_allocator.allocate_frame().unwrap();
//                 frames[(frames_start + frames_len) % frames.len()] = frame;
//                 frames_len += 1;
//                 assert!(frames_len <= frames.len(), "Too many frames");
//
//                 p3_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
//                 let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
//                 p2.zero();
//             }
//             let p2 = &mut *(p3_entry.addr().as_u64() as *mut PageTable);
//
//             let p2_entry = &mut p2[virt_addr.p2_index()];
//             if p2_entry.is_unused() {
//                 let frame = frame_allocator.allocate_frame().unwrap();
//                 frames[(frames_start + frames_len) % frames.len()] = frame;
//                 frames_len += 1;
//                 assert!(frames_len <= frames.len(), "Too many frames");
//
//                 p2_entry.set_frame(frame, PageTableFlags::PRESENT | PageTableFlags::WRITABLE);
//                 let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
//                 p1.zero();
//             }
//             let p1 = &mut *(p2_entry.addr().as_u64() as *mut PageTable);
//
//             let p1_entry = &mut p1[virt_addr.p1_index()];
//             assert!(p1_entry.is_unused());
//             p1_entry.set_frame(
//                 PhysFrame::from_start_address(PhysAddr::new(addr)).unwrap(),
//                 PageTableFlags::PRESENT | PageTableFlags::WRITABLE,
//             );
//         }
//     }
//
//     log::info!("Resetting the page table...");
//     let (_, cr3_flags) = Cr3::read();
//     Cr3::write(new_level_4_page_frame, cr3_flags);
//     log::info!("The page table is reset");
// }
