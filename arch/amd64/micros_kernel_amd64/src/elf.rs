use core::mem::size_of;
use micros_kernel_common::{ExecutableHeader, SegmentFlags, SegmentHeader};

#[allow(clippy::module_name_repetitions)]
#[repr(C)]
pub struct ElfHeader {
    ident_magic: u32,
    ident_width_class: u8,
    ident_data_endianness: u8,
    ident_version: u8,
    ident_os_abi: u8,
    ident_abi_version: u8,
    ident_padding_0: u8,
    ident_padding_1: u8,
    ident_padding_2: u8,
    ident_padding_3: u8,
    ident_padding_4: u8,
    ident_padding_5: u8,
    ident_padding_6: u8,
    file_type: u16,
    machine: u16,
    version: u32,
    entry: u64,
    program_header_offset: u64,
    section_header_offset: u64,
    flags: u32,
    elf_header_size: u16,
    program_header_entry_size: u16,
    program_header_num: u16,
    section_header_entry_size: u16,
    section_header_num: u16,
    shstrndx: u16,
}

#[allow(clippy::cast_possible_truncation)]
impl ExecutableHeader for ElfHeader {
    fn is_valid(&self, file_size: usize) -> bool {
        size_of::<ElfHeader>() <= file_size
            && self.ident_magic == ELF_MAGIC_NUMBER
            && self.ident_width_class == ELF_64_BIT
            && self.ident_data_endianness == ELF_LITTLE_ENDIAN
            && self.ident_version == 1
            && self.file_type == ELF_EXECUTABLE
            && self.machine == ELF_X86_64
            && self.program_header_offset as usize
                + self.program_header_num as usize * size_of::<ProgramHeader>()
                <= file_size
    }

    fn num_segments(&self) -> usize {
        self.program_header_num as usize
    }

    fn segment_header_table_offset(&self) -> usize {
        self.program_header_offset as usize
    }

    fn entry(&self) -> usize {
        self.entry as usize
    }
}

#[repr(C)]
pub struct ProgramHeader {
    segment_type: u32,
    flags: u32,
    offset: u64,
    virtual_address: u64,
    physical_address: u64,
    file_size: u64,
    memory_size: u64,
    align: u64,
}

#[allow(clippy::cast_possible_truncation)]
impl SegmentHeader for ProgramHeader {
    fn offset(&self) -> usize {
        self.offset as usize
    }

    fn segment_type(&self) -> u32 {
        self.segment_type
    }

    fn file_size(&self) -> usize {
        self.file_size as usize
    }

    fn memory_size(&self) -> usize {
        self.memory_size as usize
    }

    fn address(&self) -> usize {
        self.virtual_address as usize
    }

    fn flags(&self) -> SegmentFlags {
        SegmentFlags(self.flags)
    }
}

const ELF_MAGIC_NUMBER: u32 = 0x464c_457f;
const ELF_64_BIT: u8 = 2;
const ELF_LITTLE_ENDIAN: u8 = 1;
const ELF_EXECUTABLE: u16 = 2;
const ELF_X86_64: u16 = 0x3e;
