arch ?= x86_64
build_folder := target/build/debug/
kernel := $(build_folder)/kernel-$(arch).bin
iso := $(build_folder)/os-$(arch).iso
target ?= $(arch)-mxos
mxos_kernel := target/$(target)/debug/libmxos.a

linker_script := src/arch/$(arch)/linker.ld
grub_cfg := src/arch/$(arch)/grub.cfg
assembly_source_files := $(wildcard src/arch/$(arch)/*.asm)
assembly_object_files := $(patsubst src/arch/$(arch)/%.asm, \
	$(build_folder)/arch/$(arch)/%.o, $(assembly_source_files))

.PHONY: all clean run iso kernel

all: $(kernel)

clean:
	@rm -r build

run: $(iso)
	@mkdir -p $$(date +"./logs/%Y-%m-%d/")
	@ln -sf "$$(date +"$$(pwd)/logs/%Y-%m-%d/%H-%M-%S-%Z.log")" ./logs/last.log
	@qemu-system-x86_64 \
		-s -S \
		-m 8G \
		-cdrom $(iso) \
		-serial "file:$$(date +"logs/%Y-%m-%d/%H-%M-%S-%Z.log")"
	# 	-bios /usr/share/ovmf/OVMF.fd \
	# 	-drive format=raw,file=$(iso) \

# subprocess.run(['qemu-system-x86_64',
#     '-bios', Path('/usr/share/ovmf/OVMF.fd'),
#     '-drive', f'format=raw,file={bootloader_image}',
#     '-serial', f'file:{clean_serial_log_path}', # sets the serial to print into
#     '-gdb', 'tcp::1234', # accepts gdb conection ( localhost:1234 )
#     '-S', # starts suspended
#     *sys.argv[1:-1],
# ])

iso: $(iso)

$(iso): $(kernel) $(grub_cfg)
	@mkdir -p "$(build_folder)/isofiles/boot/grub"
	@cp $(kernel) "$(build_folder)/isofiles/boot/kernel.bin"
	@cp $(grub_cfg) "$(build_folder)/isofiles/boot/grub"
	@grub-mkrescue -o $(iso) "$(build_folder)/isofiles" 2> /dev/null
	@rm -r "$(build_folder)/isofiles"

$(kernel): kernel $(mxos_kernel) $(assembly_object_files) $(linker_script)
	@ld.lld -n -T $(linker_script) -o $(kernel) $(assembly_object_files) $(mxos_kernel)

kernel:
	@cargo build

# compile assembly files
$(build_folder)/arch/$(arch)/%.o: src/arch/$(arch)/%.asm
	@mkdir -p $(shell dirname $@)
	@nasm -felf64 -g $< -o $@
