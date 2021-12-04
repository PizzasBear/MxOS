//! This module contains a lot of the structures and algorithms related to memory allocation.
//!

mod btree;
mod bump;
mod slab;
mod vma;

pub use slab::{SlabAllocator, SlabBox};

use btree::BTree;
pub use bump::BumpAllocator;

use core::mem::MaybeUninit;
use core::ptr;
use core::slice;
use multiboot2::{BootInformation, MemoryMapTag};
use x86_64::registers::control::Cr3;
use x86_64::structures::paging::{
    FrameAllocator, PageSize, PageTable, PageTableFlags, PhysFrame, Size2MiB,
};
use x86_64::{PhysAddr, VirtAddr};

#[derive(Debug)]
#[repr(C, align(16))]
struct BuddyFreeList {
    ptr: usize,
    next: Option<SlabBox<BuddyFreeList>>,
}

struct Buddies {
    bitmap: &'static mut [u64],
    free_list: Option<SlabBox<BuddyFreeList>>,
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
        let order_buddy_size = self.base_size << order;

        while let Some(free_list) = self.buddies[order].free_list.take() {
            let BuddyFreeList { ptr, next } = free_list.free_move(&mut self.free_list_alloc);
            self.buddies[order].free_list = next;

            let chunk_ptr = ptr / (self.base_size << order);
            if self.is_used(order, chunk_ptr) {
                continue;
            }

            self.set_used(order, chunk_ptr);
            return Some(self.offset + ptr);
        }

        if order == self.buddies.len() - 1 {
            None
        } else {
            let ptr = self.malloc(order + 1)?;

            let chunk_ptr = (ptr - self.offset) / order_buddy_size;

            self.set_unused(order, chunk_ptr + 1);
            self.buddies[order].free_list = Some(SlabBox::new(
                &mut self.free_list_alloc,
                BuddyFreeList {
                    ptr: ptr - self.offset + order_buddy_size,
                    next: self.buddies[order].free_list.take(),
                },
            ));

            Some(ptr)
        }
    }

    pub fn free(&mut self, ptr: usize, order: usize) {
        assert!(
            self.is_unused(order, ptr),
            "Double free detected, consider yourself lucky."
        );

        let chunk_ptr = (ptr - self.offset) / (self.base_size << order);
        if order < self.buddies.len() - 1 && self.is_unused(order, chunk_ptr ^ 1) {
            self.set_used(order, chunk_ptr ^ 1);
            self.free(ptr, order + 1);
        } else {
            self.set_unused(order, chunk_ptr);
            self.buddies[order].free_list = Some(SlabBox::new(
                &mut self.free_list_alloc,
                BuddyFreeList {
                    ptr: ptr - self.offset,
                    next: self.buddies[order].free_list.take(),
                },
            ));
        }
    }

    #[inline]
    fn is_unused(&self, order: usize, chunk_ptr: usize) -> bool {
        self.buddies[order].bitmap[chunk_ptr >> 6] & 1 << (chunk_ptr & 63) == 0
    }

    #[inline]
    fn is_used(&self, order: usize, chunk_ptr: usize) -> bool {
        !self.is_unused(order, chunk_ptr)
    }

    #[inline]
    fn set_unused(&mut self, order: usize, chunk_ptr: usize) {
        self.buddies[order].bitmap[chunk_ptr >> 6] &= !(1 << (chunk_ptr & 63));
    }

    #[inline]
    fn set_used(&mut self, order: usize, chunk_ptr: usize) {
        self.buddies[order].bitmap[chunk_ptr >> 6] |= 1 << (chunk_ptr & 63);
    }

    pub unsafe fn mark_as_used(&mut self, mut start_address: usize, mut end_address: usize) {
        fn order_mark_as_used<const N: usize>(
            buddy_alloc: &mut BuddyAllocator<N>,
            mut order: usize,
            mut chunk_ptr: usize,
        ) {
            while order < N {
                if order < N - 1 && buddy_alloc.is_used(order, chunk_ptr) {
                    if buddy_alloc.is_used(order, chunk_ptr ^ 1) {
                        buddy_alloc.set_unused(order, chunk_ptr ^ 1);
                        buddy_alloc.buddies[order].free_list = Some(SlabBox::new(
                            &mut buddy_alloc.free_list_alloc,
                            BuddyFreeList {
                                ptr: (chunk_ptr ^ 1) * (buddy_alloc.base_size << order),
                                next: buddy_alloc.buddies[order].free_list.take(),
                            },
                        ));
                    }
                    order += 1;
                    chunk_ptr /= 2;
                } else {
                    buddy_alloc.set_used(order, chunk_ptr);
                    break;
                }
            }
        }

        start_address = (start_address - self.offset) / self.base_size;
        end_address = (end_address - self.offset + self.base_size - 1) / self.base_size;

        let mut order = 0;
        while order < N - 1 && start_address < end_address {
            if start_address & 1 != 0 {
                order_mark_as_used(self, order, start_address);
            }
            if end_address & 1 != 0 {
                order_mark_as_used(self, order, end_address - 1);
            }

            start_address = (start_address + 1) / 2;
            end_address /= 2;

            order += 1;
        }

        if order == N - 1 {
            for i in start_address..end_address {
                self.set_used(order, i);
            }
        }
    }
}

const GLOBAL_BUDDY_DEPTH: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
struct MemSegment {
    pub ptr: usize,
    pub size: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
struct MemSegmentOrdBySize {
    pub size: usize,
    pub ptr: usize,
}

/// The global allocator struct
pub struct GlobalChunkAllocator {
    buddy_alloc: BuddyAllocator<GLOBAL_BUDDY_DEPTH>,
    virt_addr_alloc: BTree<(usize, usize), ()>,
    pml4_table: PageTable,
    pdp_tables: &'static mut [PageTable; 512],
    chunk_checks: bool,
}

/// The global allocator static
pub static GLOBAL_CHUNK_ALLOCATOR: spin::Mutex<Option<GlobalChunkAllocator>> =
    spin::Mutex::new(None);

/// Initialize the global allocator static
pub unsafe fn init(
    kernel_start: usize,
    kernel_end: usize,
    phys_stack_frame: usize,
    boot_info: &BootInformation,
    memory_map_tag: &MemoryMapTag,
) {
    log::info!("Entered mem::init()");
    let mut global_chunk_allocator_lock = GLOBAL_CHUNK_ALLOCATOR.lock();
    assert!(global_chunk_allocator_lock.is_none());

    let mem_size = memory_map_tag
        .memory_areas()
        .map(|area| area.end_address())
        .max()
        .unwrap();
    const TOP_BLOCK_SIZE: usize = 1 << 20 + GLOBAL_BUDDY_DEPTH;

    let mem_size = mem_size as usize & !(TOP_BLOCK_SIZE - 1);
    assert!(mem_size as u64 / Size2MiB::SIZE / 8 <= Size2MiB::SIZE / 2);

    log::info!("Creating bump_allocator");
    let mut bump_allocator = BumpAllocator::new(
        [
            kernel_start..kernel_end,
            boot_info.start_address()..boot_info.end_address(),
            phys_stack_frame..phys_stack_frame + 0x200000,
        ],
        memory_map_tag,
    );

    let buddies_frame = bump_allocator
        .allocate_frame()
        .expect("Couldn't allocate a frame for the buddies");
    log::info!(
        "Allocated chunk=0x{:x} for buddy allocator",
        buddies_frame.start_address().as_u64()
    );
    let free_list_alloc_frame = bump_allocator
        .allocate_frame()
        .expect("Couldn't allocate a frame for the buddies' free list slab allocator");
    log::info!(
        "Allocated chunk=0x{:x} for free list allocator",
        free_list_alloc_frame.start_address().as_u64()
    );

    let free_list_alloc = SlabAllocator::new(slice::from_raw_parts_mut(
        free_list_alloc_frame.start_address().as_u64() as _,
        Size2MiB::SIZE as _,
    ));

    log::info!("Creating buddy_alloc");
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
        top_buddies.free_list = Some(SlabBox::new(
            &mut buddy_alloc.free_list_alloc,
            BuddyFreeList {
                ptr: (TOP_BLOCK_SIZE * i) as _,
                next: top_buddies.free_list.take(),
            },
        ));
    }
    assert!(
        kernel_start & !0x1fffff < boot_info.end_address() + 0x1fffff & !0x1fffff
            && boot_info.start_address() & !0x1fffff < kernel_end + 0x1fffff & !0x1fffff
    );
    buddy_alloc.mark_as_used(
        kernel_start.min(boot_info.start_address()),
        kernel_end.max(boot_info.end_address()),
    );
    buddy_alloc.mark_as_used(
        buddies_frame.start_address().as_u64() as _,
        (buddies_frame.start_address().as_u64() + buddies_frame.size()) as _,
    );
    buddy_alloc.mark_as_used(
        free_list_alloc_frame.start_address().as_u64() as _,
        (free_list_alloc_frame.start_address().as_u64() + free_list_alloc_frame.size()) as _,
    );
    buddy_alloc.mark_as_used(phys_stack_frame, phys_stack_frame + 0x200000);
    if 0x200000 <= kernel_start {
        buddy_alloc.mark_as_used(0, 0x200000);
    }

    let virt_addr_alloc_chunk = buddy_alloc.malloc(0).unwrap();
    log::info!(
        "Allocated chunk=0x{:x} for virtual address allocator",
        virt_addr_alloc_chunk
    );

    log::info!("Creating pml4_table");
    let mut pml4_table = PageTable::new();
    let pdp_tables_addr = buddy_alloc.malloc(0).unwrap();
    let pdp_tables = &mut *(pdp_tables_addr as *mut [PageTable; 512]);

    for i in 0..512 {
        ptr::write((pdp_tables_addr as *mut PageTable).add(i), PageTable::new());
    }
    // let pdp_tables = &mut *(pdp_tables_phys_addr as *mut [PageTable; 512]);
    for (i, entry) in pml4_table.iter_mut().enumerate() {
        entry.set_addr(
            PhysAddr::new((pdp_tables_addr + 4096 * i) as _),
            PageTableFlags::WRITABLE | PageTableFlags::PRESENT,
        );
    }

    {
        let mut map_addresses = [
            (
                boot_info.start_address().min(kernel_start) & !0x1fffff,
                (boot_info.end_address().max(kernel_end) + 0x1fffff & !0x1fffff)
                    - (boot_info.start_address().min(kernel_start) & !0x1fffff),
                boot_info.start_address().min(kernel_start) & !0x1fffff,
            ),
            (
                buddies_frame.start_address().as_u64() as usize,
                buddies_frame.size() as usize,
                buddies_frame.start_address().as_u64() as usize,
            ),
            (
                free_list_alloc_frame.start_address().as_u64() as usize,
                free_list_alloc_frame.size() as usize,
                free_list_alloc_frame.start_address().as_u64() as usize,
            ),
            (virt_addr_alloc_chunk, 0x200000, virt_addr_alloc_chunk),
            (pdp_tables_addr, 0x200000, pdp_tables_addr),
            (
                phys_stack_frame,
                0x200000,
                (511 << 39) | (510 << 30) | (1 << 21) | (0xffff << 48),
            ),
            (0, 0, 0),
            (0, 0, 0),
            (0, 0, 0),
            (0, 0, 0),
            (0, 0, 0),
            (0, 0, 0),
        ];
        let mut ptr = 0;
        let mut end = 6;

        while ptr != end {
            let (phys_addr, size, virt_addr) = map_addresses[ptr];
            ptr = (ptr + 1) % map_addresses.len();

            for i in (0..size).step_by(0x200000) {
                let phys_addr = PhysAddr::new((phys_addr + i) as _);
                let virt_addr = VirtAddr::new((virt_addr + i) as _);

                let pdp_table = &mut pdp_tables[usize::from(virt_addr.p4_index())];

                let pd_addr = if pdp_table[virt_addr.p3_index()].is_unused() {
                    let pd_addr = if pdp_table[0].is_unused() {
                        let pd0_addr = buddy_alloc.malloc(0).unwrap() as u64;
                        map_addresses[end] = (
                            pd0_addr as _,
                            0x200000,
                            (511 << 39)
                                | (511 << 30)
                                | (usize::from(virt_addr.p4_index()) << 21)
                                | (0xffff << 48),
                        );
                        end = (end + 1) % map_addresses.len();

                        pdp_table[0].set_addr(PhysAddr::new(pd0_addr), PageTableFlags::WRITABLE);
                        PhysAddr::new(pd0_addr + 4096 * u64::from(virt_addr.p3_index()))
                    } else {
                        let pd0_addr = pdp_table[0].addr();

                        pd0_addr + 4096 * u64::from(virt_addr.p3_index())
                    };

                    ptr::write(pd_addr.as_u64() as *mut _, PageTable::new());

                    pdp_table[virt_addr.p3_index()]
                        .set_addr(pd_addr, PageTableFlags::WRITABLE | PageTableFlags::PRESENT);

                    pd_addr.as_u64() as usize
                } else {
                    pdp_table[virt_addr.p3_index()].addr().as_u64() as usize
                };

                let pd_table = &mut *(pd_addr as *mut PageTable);

                assert!(pd_table[virt_addr.p2_index()].is_unused());

                pd_table[virt_addr.p2_index()].set_addr(
                    phys_addr,
                    PageTableFlags::HUGE_PAGE | PageTableFlags::WRITABLE | PageTableFlags::PRESENT,
                );
            }
        }
    }

    let mut virt_addr_alloc = BTree::new(slice::from_raw_parts_mut(
        virt_addr_alloc_chunk as _,
        buddy_alloc.base_size,
    ));

    {
        let mut virt_start_addresses = [
            boot_info.start_address().min(kernel_start) & !0x1fffff,
            buddies_frame.start_address().as_u64() as usize,
            free_list_alloc_frame.start_address().as_u64() as usize,
            virt_addr_alloc_chunk,
            pdp_tables_addr,
            (1 << 48) - (2 << 30),
        ];
        let mut virt_end_addresses = [
            boot_info.end_address().max(kernel_end) + 0x1fffff & !0x1fffff,
            (buddies_frame.start_address().as_u64() + buddies_frame.size()) as usize,
            (free_list_alloc_frame.start_address().as_u64() + free_list_alloc_frame.size())
                as usize,
            virt_addr_alloc_chunk + buddy_alloc.base_size,
            pdp_tables_addr + 0x200000,
            (1 << 48),
        ];
        virt_start_addresses.sort_unstable();
        virt_end_addresses.sort_unstable();

        let mut i = 0;
        let mut j = 0;
        let mut depth = 0;
        let mut last_end = 0x200000;

        while i < virt_start_addresses.len() && j < virt_end_addresses.len() {
            if virt_start_addresses[i] < virt_end_addresses[j] {
                if depth == 0 && last_end < virt_start_addresses[i] {
                    assert!(virt_addr_alloc
                        .insert(
                            (virt_start_addresses[i] - last_end, virt_start_addresses[i]),
                            ()
                        )
                        .is_none());
                }
                depth += 1;
                i += 1;
            } else if virt_end_addresses[j] < virt_start_addresses[i] {
                last_end = virt_end_addresses[j];

                depth -= 1;
                j += 1;
            } else {
                i += 1;
                j += 1;
            }
        }
    }

    let global_chunk_allocator = global_chunk_allocator_lock.insert(GlobalChunkAllocator {
        buddy_alloc,
        virt_addr_alloc,
        pml4_table,
        pdp_tables,
        chunk_checks: true,
    });

    log::info!("Initialized GLOBAL_CHUNK_ALLOCATOR");

    let (_, cr3_flags) = Cr3::read();
    Cr3::write(
        PhysFrame::from_start_address(PhysAddr::new(
            &global_chunk_allocator.pml4_table as *const PageTable as _,
        ))
        .unwrap(),
        cr3_flags,
    );

    log::info!("Initialized allocator paging");
}

impl GlobalChunkAllocator {
    const SUPER_PD_TABLE: *mut PageTable =
        ((511 << 39) | (511 << 30) | (511 << 21) | (511 << 12) | (0xffff << 48)) as *mut _;

    fn virt_alloc(&mut self, size: usize) -> usize {
        let mut entry = self.virt_addr_alloc.get_entry(&(size, 0)).unwrap_err();
        if entry.key().0 < size {
            assert!(entry.next());
        }

        let key = *entry.key();

        drop(entry);
        assert!(size <= key.0);
        self.virt_addr_alloc.remove(&key);
        if size < key.0 {
            self.virt_addr_alloc
                .insert((key.0 - size, key.1 + size), ());
        }

        key.1
    }

    /// Allocates a chunk of size `2MiB * 2^order`. `order` has to be smaller than 8. The function
    /// returns the chunk.
    pub unsafe fn malloc(&mut self, order: usize) -> &'static mut [u8] {
        if self.chunk_checks {
            self.chunk_checks = false;
            while self.virt_addr_alloc.needs_new_chunk() {
                let chunk = self.malloc(0);
                self.virt_addr_alloc.add_chunk(chunk);
            }
            while self.buddy_alloc.free_list_alloc.needs_new_chunk() {
                let chunk = self.malloc(0);
                self.buddy_alloc.free_list_alloc.add_chunk(chunk);
            }
            self.chunk_checks = true;
        }

        let phys_addr = PhysAddr::new(self.buddy_alloc.malloc(order).unwrap() as _);
        let virt_addr = VirtAddr::new_truncate(self.virt_alloc(0x200000 << order) as _);

        for i in (0..0x200000usize << order).step_by(0x200000) {
            let phys_addr = phys_addr + i;
            let virt_addr = virt_addr + i;

            let pdp_table = &mut self.pdp_tables[usize::from(virt_addr.p4_index())];

            let pd_addr = (511 << 39)
                | (511 << 30)
                | (usize::from(virt_addr.p4_index()) << 21)
                | (usize::from(virt_addr.p3_index()) << 12);

            if pdp_table[virt_addr.p3_index()].is_unused() {
                let phys_pd_addr = if pdp_table[0].is_unused() {
                    let pd0_addr = self.buddy_alloc.malloc(0).unwrap() as u64;

                    (*Self::SUPER_PD_TABLE)[virt_addr.p4_index()].set_addr(
                        PhysAddr::new(pd0_addr),
                        PageTableFlags::HUGE_PAGE
                            | PageTableFlags::WRITABLE
                            | PageTableFlags::PRESENT,
                    );

                    pdp_table[0].set_addr(PhysAddr::new(pd0_addr), PageTableFlags::WRITABLE);
                    PhysAddr::new(pd0_addr + 4096 * u64::from(virt_addr.p3_index()))
                } else {
                    let pd0_addr = pdp_table[0].addr();

                    pd0_addr + 4096 * u64::from(virt_addr.p3_index())
                };

                ptr::write(pd_addr as *mut _, PageTable::new());

                pdp_table[virt_addr.p3_index()].set_addr(
                    phys_pd_addr,
                    PageTableFlags::WRITABLE | PageTableFlags::PRESENT,
                );
            }

            let pd_table = &mut *(pd_addr as *mut PageTable);

            assert!(pd_table[virt_addr.p2_index()].is_unused());

            pd_table[virt_addr.p2_index()].set_addr(
                phys_addr,
                PageTableFlags::HUGE_PAGE | PageTableFlags::WRITABLE | PageTableFlags::PRESENT,
            );
        }

        slice::from_raw_parts_mut(virt_addr.as_u64() as _, 0x200000 << order)
    }
}
