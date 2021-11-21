# MxOS
MxOS is an exmperimental OS written in Rust.
MxOS's goal is to be a highly asynchronious microkernel OS, with good security.

Currently the kernel chunk allocator is built.
It is going to be a buddy allocator for physical memory (2MiB -> 256MiB).
MxOS will use a B-Tree for the virtual memory allocator, borrowed from [DStruct](https://github.com/PizzasBear/DStruct) and modified a bit.
The nodes and leafs of the B-Tree are allocated from their own slab allocators.

To compile MxOS you need:
  - Linux (or potentially *BSD)
  - GRUB2
  - GNU xorriso
  - Cargo & Rust nightly
  - Netwide Assembler (NASM)
  - QEMU
  - GNU Make
  - Python (For the experiments)
