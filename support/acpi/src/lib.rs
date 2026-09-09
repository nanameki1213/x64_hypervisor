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

pub fn root_system_description_pointer_verification(rsdp_ptr: *const RootSystemDescriptionPointer) -> bool {
    let rsdp = unsafe {
        &*rsdp_ptr
    };

    let is_correct_signature =
        rsdp.signature[0] == 'R' as u8 &&
        rsdp.signature[1] == 'S' as u8 &&
        rsdp.signature[2] == 'D' as u8 &&
        rsdp.signature[3] == ' ' as u8 &&
        rsdp.signature[4] == 'P' as u8 &&
        rsdp.signature[5] == 'T' as u8 &&
        rsdp.signature[6] == 'R' as u8 &&
        rsdp.signature[7] == ' ' as u8;
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
        let rsdp_bytes = unsafe {
            core::slice::from_raw_parts(rsdp_ptr as *const u8, rsdp.length as usize)
        };
        let mut sum = 0;
        for byte in rsdp_bytes {
            sum += *byte as usize;
        }

        if sum & 0xff != 0 {
            return false;
        }
    }

    return true;
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

    return false;
}

pub fn calclate_checksum<T: AcpiTable>(table: &T) -> bool {
    let header = table.header();

    let header_bytes = unsafe {
        &*core::slice::from_raw_parts(&raw const header as *const u8, header.length as usize)
    };

    let mut sum = 0;
    for byte in header_bytes {
        sum += byte;
    }

    if sum != 0 {
        return false;
    }

    return true;
}
