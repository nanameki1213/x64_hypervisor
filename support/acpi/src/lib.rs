#![no_std]

#[derive(Debug)]
#[repr(packed)]
#[repr(C)]
pub struct RootSystemDescriptionPointer {
    pub signature: [u8; 8],
    pub checksum: u8,
    pub oemid: [u8; 6],
    pub revision: u8,
    pub rsdt_address: u32,
    pub length: u32,
    pub xsdt_address: u64,
    pub extended_checksum: u8,
    pub reserved: [u8; 3],
}

/// # Safety
///
/// `rsdp_ptr` must point to a valid, readable RSDP structure.
pub unsafe fn root_system_description_pointer_verification(
    rsdp_ptr: *const RootSystemDescriptionPointer,
) -> bool {
    let rsdp = unsafe { &*rsdp_ptr };

    let is_correct_signature = rsdp.signature[0] == b'R'
        && rsdp.signature[1] == b'S'
        && rsdp.signature[2] == b'D'
        && rsdp.signature[3] == b' '
        && rsdp.signature[4] == b'P'
        && rsdp.signature[5] == b'T'
        && rsdp.signature[6] == b'R'
        && rsdp.signature[7] == b' ';
    if !is_correct_signature {
        return false;
    }

    let mut sum = 0;
    let mut byte = rsdp_ptr as *const u8;
    while byte < &raw const (rsdp.length) as *mut u8 {
        unsafe {
            sum += *byte as usize;
            byte = byte.add(1);
        }
    }

    if sum & 0xff != 0 {
        return false;
    }

    if rsdp.revision >= 2 {
        let rsdp_bytes =
            unsafe { core::slice::from_raw_parts(rsdp_ptr as *const u8, rsdp.length as usize) };
        let mut sum = 0;
        for byte in rsdp_bytes {
            sum += *byte as usize;
        }

        if sum & 0xff != 0 {
            return false;
        }
    }

    true
}

#[repr(packed)]
#[repr(C)]
pub struct AcpiTableHeader {
    pub signature: [u8; 4],
    pub length: u32,
    pub revision: u8,
    pub checksum: u8,
    pub oem_id: [u8; 6],
    pub oem_table_id: [u8; 8],
    pub oem_revision: u32,
    pub asl_compiler_id: [u8; 4],
    pub asl_compiler_revision: u32,
}

pub trait AcpiTable {
    const SIGNATURE: &'static str;
    fn header(&self) -> &AcpiTableHeader;
}

#[repr(packed)]
#[repr(C)]
pub struct Xsdt {
    pub header: AcpiTableHeader,
    pub entry: *const u64,
}

impl AcpiTable for Xsdt {
    const SIGNATURE: &'static str = "XSDT";
    fn header(&self) -> &AcpiTableHeader {
        &self.header
    }
}

#[repr(packed)]
#[repr(C)]
pub struct Fadt {
    pub header: AcpiTableHeader,
    pub firmware_ctrl: u32,
    pub dsdt: u32,
    // ignore under this field
}

impl AcpiTable for Fadt {
    const SIGNATURE: &'static str = "FADT";
    fn header(&self) -> &AcpiTableHeader {
        &self.header
    }
}

#[repr(packed)]
#[repr(C)]
pub struct Dsdt {
    pub header: AcpiTableHeader,
}

impl AcpiTable for Dsdt {
    const SIGNATURE: &'static str = "DSDT";
    fn header(&self) -> &AcpiTableHeader {
        &self.header
    }
}

pub fn is_valid_signature<T: AcpiTable>(table: &T) -> bool {
    let header = table.header();

    let signature = T::SIGNATURE.as_bytes();
    if header.signature == signature {
        return true;
    }

    false
}

pub fn calclate_checksum<T: AcpiTable>(table: &T) -> bool {
    let header = table.header();

    let header_bytes = unsafe {
        core::slice::from_raw_parts(
            header as *const AcpiTableHeader as *const u8,
            header.length as usize,
        )
    };

    let mut sum: u8 = 0;
    for &byte in header_bytes {
        sum = sum.wrapping_add(byte);
    }

    if sum != 0 {
        return false;
    }

    true
}

impl Xsdt {
    /// XSDT が指すテーブルの物理アドレス（エントリは 8 バイトだが、8 バイト境界にあるとは限らない）
    pub fn entries(&self) -> impl Iterator<Item = u64> + '_ {
        let base = &self.header as *const AcpiTableHeader as *const u8;
        let header_size = core::mem::size_of::<AcpiTableHeader>();
        let count = (self.header.length as usize).saturating_sub(header_size) / 8;
        (0..count).map(move |i| unsafe {
            core::ptr::read_unaligned(base.add(header_size + i * 8) as *const u64)
        })
    }

    /// シグネチャが `T::SIGNATURE` で、チェックサムが正しい最初のテーブル
    ///
    /// # Safety
    /// XSDT のエントリが、読めるメモリ上の ACPI テーブルを指していること
    pub unsafe fn find<T: AcpiTable>(&self) -> Option<&T> {
        self.entries()
            .map(|addr| unsafe { &*(addr as *const T) })
            .find(|t| is_valid_signature(*t) && calclate_checksum(*t))
    }
}

/// MADT（Multiple APIC Description Table）。ヘッダの後ろに可変長のエントリが続く
#[repr(packed)]
#[repr(C)]
pub struct Madt {
    pub header: AcpiTableHeader,
    pub local_apic_address: u32,
    pub flags: u32,
}

impl AcpiTable for Madt {
    const SIGNATURE: &'static str = "APIC";
    fn header(&self) -> &AcpiTableHeader {
        &self.header
    }
}

/// MADT のエントリのうち、HV が使うもの
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MadtEntry {
    /// Processor Local APIC（type 0）
    LocalApic { uid: u8, apic_id: u8, flags: u32 },
    /// Processor Local x2APIC（type 9）
    LocalX2Apic {
        uid: u32,
        x2apic_id: u32,
        flags: u32,
    },
    /// それ以外（I/O APIC など）
    Other { entry_type: u8 },
}

/// Local APIC / x2APIC の flags
pub const MADT_LAPIC_ENABLED: u32 = 1 << 0;
pub const MADT_LAPIC_ONLINE_CAPABLE: u32 = 1 << 1;

impl Madt {
    pub fn entries(&self) -> impl Iterator<Item = MadtEntry> + '_ {
        let base = self as *const Madt as *const u8;
        let end = self.header.length as usize;
        let mut offset = core::mem::size_of::<Madt>();
        core::iter::from_fn(move || {
            if offset + 2 > end {
                return None;
            }
            let read = |o: usize| unsafe { *base.add(offset + o) };
            let read_u32 =
                |o: usize| unsafe { core::ptr::read_unaligned(base.add(offset + o) as *const u32) };
            let (entry_type, len) = (read(0), read(1) as usize);
            if len < 2 || offset + len > end {
                return None;
            }
            let entry = match (entry_type, len) {
                (0, 8) => MadtEntry::LocalApic {
                    uid: read(2),
                    apic_id: read(3),
                    flags: read_u32(4),
                },
                (9, 16) => MadtEntry::LocalX2Apic {
                    x2apic_id: read_u32(4),
                    flags: read_u32(8),
                    uid: read_u32(12),
                },
                _ => MadtEntry::Other { entry_type },
            };
            offset += len;
            Some(entry)
        })
    }

    /// 使える（有効か、online capable な）CPU の APIC ID
    pub fn apic_ids(&self) -> impl Iterator<Item = u32> + '_ {
        let usable = |flags: u32| flags & (MADT_LAPIC_ENABLED | MADT_LAPIC_ONLINE_CAPABLE) != 0;
        self.entries().filter_map(move |e| match e {
            MadtEntry::LocalApic { apic_id, flags, .. } if usable(flags) => Some(apic_id as u32),
            MadtEntry::LocalX2Apic {
                x2apic_id, flags, ..
            } if usable(flags) => Some(x2apic_id),
            _ => None,
        })
    }
}
