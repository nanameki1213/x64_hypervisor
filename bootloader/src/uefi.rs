#![allow(dead_code)]

use core::ffi::c_void;

// ──────────────────────────────────────────────────────────────────────────────
// Primitive type aliases
// ──────────────────────────────────────────────────────────────────────────────

/// EFI_STATUS — UEFI ステータスコード一覧 (UEFI Spec 2.10 Appendix D)
///
/// エラーコードは最上位ビット (bit 63) が立っている。
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EfiStatus {
    // ── 成功 ──────────────────────────────────────────────────────────────────
    Success = 0,

    // ── 警告 (bit63 = 0) ──────────────────────────────────────────────────────
    WarnUnknownGlyph = 1,
    WarnDeleteFailure = 2,
    WarnWriteFailure = 3,
    WarnBufferTooSmall = 4,
    WarnStaleData = 5,
    WarnFileSystem = 6,
    WarnResetRequired = 7,

    // ── エラー (bit63 = 1) ────────────────────────────────────────────────────
    LoadError = 0x8000_0000_0000_0001,
    InvalidParameter = 0x8000_0000_0000_0002,
    Unsupported = 0x8000_0000_0000_0003,
    BadBufferSize = 0x8000_0000_0000_0004,
    BufferTooSmall = 0x8000_0000_0000_0005,
    NotReady = 0x8000_0000_0000_0006,
    DeviceError = 0x8000_0000_0000_0007,
    WriteProtected = 0x8000_0000_0000_0008,
    OutOfResources = 0x8000_0000_0000_0009,
    VolumeCorrupted = 0x8000_0000_0000_000A,
    VolumeFull = 0x8000_0000_0000_000B,
    NoMedia = 0x8000_0000_0000_000C,
    MediaChanged = 0x8000_0000_0000_000D,
    NotFound = 0x8000_0000_0000_000E,
    AccessDenied = 0x8000_0000_0000_000F,
    NoResponse = 0x8000_0000_0000_0010,
    NoMapping = 0x8000_0000_0000_0011,
    Timeout = 0x8000_0000_0000_0012,
    NotStarted = 0x8000_0000_0000_0013,
    AlreadyStarted = 0x8000_0000_0000_0014,
    Aborted = 0x8000_0000_0000_0015,
    IcmpError = 0x8000_0000_0000_0016,
    TftpError = 0x8000_0000_0000_0017,
    ProtocolError = 0x8000_0000_0000_0018,
    IncompatibleVersion = 0x8000_0000_0000_0019,
    SecurityViolation = 0x8000_0000_0000_001A,
    CrcError = 0x8000_0000_0000_001B,
    EndOfMedia = 0x8000_0000_0000_001C,
    EndOfFile = 0x8000_0000_0000_001F,
    InvalidLanguage = 0x8000_0000_0000_0020,
    CompromisedData = 0x8000_0000_0000_0021,
    HttpError = 0x8000_0000_0000_0023,
}

impl EfiStatus {
    /// エラーコード（bit63 = 1）かどうかを返す
    #[inline]
    pub fn is_error(self) -> bool {
        (self as usize) & (1 << 63) != 0
    }

    /// 警告コード（0 < value < ERROR_BIT）かどうかを返す
    #[inline]
    pub fn is_warning(self) -> bool {
        let v = self as usize;
        v != 0 && v & (1 << 63) == 0
    }
}

/// EFI_EVENT
pub type EfiEvent = *mut c_void;

/// EFI_HANDLE
pub type EfiHandle = *mut c_void;

/// EFI_TPL (Task Priority Level)
pub type EfiTpl = usize;

pub type EfiPhysicalAddress = u64;
pub type EfiVirtualAddress = u64;

// ──────────────────────────────────────────────────────────────────────────────
// EFI_GUID
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

// ──────────────────────────────────────────────────────────────────────────────
// Well-known Protocol GUIDs (UEFI Spec 2.10)
// ──────────────────────────────────────────────────────────────────────────────

// ── Console ───────────────────────────────────────────────────────────────────

/// EFI_SIMPLE_TEXT_INPUT_PROTOCOL_GUID
/// SystemTable->ConIn のショートカットでも同じプロトコルを参照している。
pub static EFI_SIMPLE_TEXT_INPUT_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x387477c1,
    data2: 0x69c7,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

/// EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL_GUID
/// SystemTable->ConOut / StdErr のショートカットでも同じプロトコルを参照している。
pub static EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x387477c2,
    data2: 0x69c7,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

// ── Image / Device Path ───────────────────────────────────────────────────────

/// EFI_LOADED_IMAGE_PROTOCOL_GUID
pub static EFI_LOADED_IMAGE_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x5b1b31a1,
    data2: 0x9562,
    data3: 0x11d2,
    data4: [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

/// EFI_DEVICE_PATH_PROTOCOL_GUID
pub static EFI_DEVICE_PATH_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x09576e91,
    data2: 0x6d3f,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

// ── Storage ───────────────────────────────────────────────────────────────────

/// EFI_BLOCK_IO_PROTOCOL_GUID
pub static EFI_BLOCK_IO_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x964e5b21,
    data2: 0x6459,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

/// EFI_DISK_IO_PROTOCOL_GUID
pub static EFI_DISK_IO_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0xce345171,
    data2: 0xba0b,
    data3: 0x11d2,
    data4: [0x8e, 0x4f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

/// EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID
pub static EFI_SIMPLE_FILE_SYSTEM_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x964e5b22,
    data2: 0x6459,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

// ── Graphics ──────────────────────────────────────────────────────────────────

/// EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID
pub static EFI_GRAPHICS_OUTPUT_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x9042a9de,
    data2: 0x23dc,
    data3: 0x4a38,
    data4: [0x96, 0xfb, 0x7a, 0xde, 0xd0, 0x80, 0x51, 0x6a],
};

// ── Serial ────────────────────────────────────────────────────────────────────

/// EFI_SERIAL_IO_PROTOCOL_GUID
pub static EFI_SERIAL_IO_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0xbb25cf6f,
    data2: 0xf1d4,
    data3: 0x11d2,
    data4: [0x9a, 0x0c, 0x00, 0x90, 0x27, 0x3f, 0xc1, 0xfd],
};

// ──────────────────────────────────────────────────────────────────────────────
// EFI_TABLE_HEADER
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiTableHeader {
    pub signature: u64,
    pub revision: u32,
    pub header_size: u32,
    pub crc32: u32,
    pub reserved: u32,
}

// ──────────────────────────────────────────────────────────────────────────────
// Memory types
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub enum EfiAllocateType {
    AllocateAnyPages,
    AllocateMaxAddress,
    AllocateAddress,
    MaxAllocateType,
}

#[repr(C)]
pub enum EfiMemoryType {
    ReservedMemoryType,
    LoaderCode,
    LoaderData,
    BootServicesCode,
    BootServicesData,
    RuntimeServicesCode,
    RuntimeServicesData,
    ConventionalMemory,
    UnusableMemory,
    AcpiReclaimMemory,
    AcpiMemoryNvs,
    MemoryMappedIo,
    MemoryMappedIoPortSpace,
    PalCode,
    PersistentMemory,
    MaxMemoryType,
}

/// EFI_MEMORY_DESCRIPTOR
#[repr(C)]
pub struct EfiMemoryDescriptor {
    pub memory_type: u32,
    pub physical_start: EfiPhysicalAddress,
    pub virtual_start: EfiVirtualAddress,
    pub number_of_pages: u64,
    pub attribute: u64,
}

// ──────────────────────────────────────────────────────────────────────────────
// Event / Timer types
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub enum EfiTimerDelay {
    TimerCancel,
    TimerPeriodic,
    TimerRelative,
}

pub type EfiEventNotify = unsafe extern "efiapi" fn(event: EfiEvent, context: *mut c_void);

// ──────────────────────────────────────────────────────────────────────────────
// Protocol handler types
// ──────────────────────────────────────────────────────────────────────────────

/// `OpenProtocol` の `attributes` に渡すフラグ (UEFI Spec 2.10 §7.3)
pub mod open_protocol {
    /// ハンドルが指定プロトコルをサポートするか調べる（`HandleProtocol` の代替）
    pub const BY_HANDLE_PROTOCOL: u32 = 0x00000001;
    /// プロトコルインターフェースのポインタを取得する（ドライバ管理外）
    pub const GET_PROTOCOL: u32 = 0x00000002;
    /// プロトコルが存在するかテストする（インターフェース取得なし）
    pub const TEST_PROTOCOL: u32 = 0x00000004;
    /// 子コントローラとしてオープンする
    pub const BY_CHILD_CONTROLLER: u32 = 0x00000008;
    /// ドライバとしてオープンする
    pub const BY_DRIVER: u32 = 0x00000010;
    /// 排他的にオープンする（他のドライバを切断する）
    pub const EXCLUSIVE: u32 = 0x00000020;
}

#[repr(C)]
pub enum EfiInterfaceType {
    NativeInterface,
}

#[repr(C)]
pub enum EfiLocateSearchType {
    AllHandles,
    ByRegisterNotify,
    ByProtocol,
}

#[repr(C)]
pub struct EfiOpenProtocolInformationEntry {
    pub agent_handle: EfiHandle,
    pub controller_handle: EfiHandle,
    pub attributes: u32,
    pub open_count: u32,
}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_DEVICE_PATH_PROTOCOL (opaque header)
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiDevicePath {
    pub r#type: u8,
    pub sub_type: u8,
    pub length: [u8; 2],
}

// ──────────────────────────────────────────────────────────────────────────────
// Time
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub pad1: u8,
    pub nanosecond: u32,
    pub time_zone: i16,
    pub daylight: u8,
    pub pad2: u8,
}

#[repr(C)]
pub struct EfiTimeCapabilities {
    pub resolution: u32,
    pub accuracy: u32,
    pub sets_to_zero: bool,
}

// ──────────────────────────────────────────────────────────────────────────────
// Reset / Capsule types
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub enum EfiResetType {
    ResetCold,
    ResetWarm,
    ResetShutdown,
    ResetPlatformSpecific,
}

#[repr(C)]
pub struct EfiCapsuleHeader {
    pub capsule_guid: EfiGuid,
    pub header_size: u32,
    pub flags: u32,
    pub capsule_image_size: u32,
}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_CONFIGURATION_TABLE
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiConfigurationTable {
    pub vendor_guid: EfiGuid,
    pub vendor_table: *mut c_void,
}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_SIMPLE_TEXT_INPUT_PROTOCOL
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiInputKey {
    pub scan_code: u16,
    pub unicode_char: u16,
}

#[repr(C)]
pub struct EfiSimpleTextInputProtocol {
    pub reset: unsafe extern "efiapi" fn(this: *mut Self, extended_verify: bool) -> EfiStatus,

    pub read_key_stroke:
        unsafe extern "efiapi" fn(this: *mut Self, key: *mut EfiInputKey) -> EfiStatus,

    pub wait_for_key: EfiEvent,
}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_SIMPLE_TEXT_OUTPUT_PROTOCOL
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiSimpleTextOutputMode {
    pub max_mode: i32,
    pub mode: i32,
    pub attribute: i32,
    pub cursor_column: i32,
    pub cursor_row: i32,
    pub cursor_visible: bool,
}

#[repr(C)]
pub struct EfiSimpleTextOutputProtocol {
    pub reset: unsafe extern "efiapi" fn(this: *mut Self, extended_verify: bool) -> EfiStatus,

    pub output_string: unsafe extern "efiapi" fn(this: *mut Self, string: *const u16) -> EfiStatus,

    pub test_string: unsafe extern "efiapi" fn(this: *mut Self, string: *const u16) -> EfiStatus,

    pub query_mode: unsafe extern "efiapi" fn(
        this: *mut Self,
        mode_number: usize,
        columns: *mut usize,
        rows: *mut usize,
    ) -> EfiStatus,

    pub set_mode: unsafe extern "efiapi" fn(this: *mut Self, mode_number: usize) -> EfiStatus,

    pub set_attribute: unsafe extern "efiapi" fn(this: *mut Self, attribute: usize) -> EfiStatus,

    pub clear_screen: unsafe extern "efiapi" fn(this: *mut Self) -> EfiStatus,

    pub set_cursor_position:
        unsafe extern "efiapi" fn(this: *mut Self, column: usize, row: usize) -> EfiStatus,

    pub enable_cursor: unsafe extern "efiapi" fn(this: *mut Self, visible: bool) -> EfiStatus,

    pub mode: *mut EfiSimpleTextOutputMode,
}

// UEFI はシングルスレッド環境であり、実際にはスレッド間共有は発生しない。
// raw pointer フィールドにより Sync が自動実装されないため手動で宣言する。
unsafe impl Sync for EfiSimpleTextOutputProtocol {}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_SIMPLE_FILE_SYSTEM_PROTOCOL
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiFileIoToken {
    event: EfiEvent,
    status: EfiStatus,
    buffer_size: usize,
    buffer: *mut c_void,
}

/// EFI_FILE_INFO_ID — EfiFileProtocol::get_info / set_info で使用する GUID
pub static EFI_FILE_INFO_ID: EfiGuid = EfiGuid {
    data1: 0x09576e92,
    data2: 0x6d3f,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

/// EFI_FILE_SYSTEM_INFO_ID
pub static EFI_FILE_SYSTEM_INFO_ID: EfiGuid = EfiGuid {
    data1: 0x09576e93,
    data2: 0x6d3f,
    data3: 0x11d2,
    data4: [0x8e, 0x39, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

#[repr(C)]
pub struct EfiFileInfo {
    pub size: u64,
    pub file_size: u64,
    pub physical_size: u64,
    pub create_time: EfiTime,
    pub last_access_time: EfiTime,
    pub modification_time: EfiTime,
    pub attribute: u64,
}

#[repr(C)]
pub struct EfiFileProtocol {
    pub revision: u64,
    pub open: unsafe extern "efiapi" fn(
        this: *mut Self,
        new_handle: *mut *mut Self,
        file_name: *const u16,
        open_mode: u64,
        attributes: u64,
    ) -> EfiStatus,
    pub close: unsafe extern "efiapi" fn(this: *mut Self) -> EfiStatus,
    pub delete: unsafe extern "efiapi" fn(this: *mut Self) -> EfiStatus,
    pub read: unsafe extern "efiapi" fn(
        this: *mut Self,
        buffer_size: *mut usize,
        buffer: *mut c_void,
    ) -> EfiStatus,
    pub write: unsafe extern "efiapi" fn(
        this: *mut Self,
        buffer_size: *mut usize,
        buffer: *mut c_void,
    ) -> EfiStatus,
    pub get_position: unsafe extern "efiapi" fn(this: *mut Self, position: *mut u64) -> EfiStatus,
    pub set_position: unsafe extern "efiapi" fn(this: *mut Self, position: u64) -> EfiStatus,
    pub get_info: unsafe extern "efiapi" fn(
        this: *mut Self,
        information_type: *const EfiGuid,
        buffer_size: *mut usize,
        buffer: *mut c_void,
    ) -> EfiStatus,
    pub set_info: unsafe extern "efiapi" fn(
        this: *mut Self,
        information_type: EfiGuid,
        buffer_size: usize,
        buffer: *mut c_void,
    ) -> EfiStatus,
    pub flush: unsafe extern "efiapi" fn(this: *mut Self) -> EfiStatus,
    pub open_ex: unsafe extern "efiapi" fn(
        this: *mut Self,
        new_handle: *mut *mut Self,
        file_name: *const u16,
        open_mode: u64,
        attributes: u64,
        token: *mut EfiFileIoToken,
    ) -> EfiStatus,
    pub read_ex:
        unsafe extern "efiapi" fn(this: *mut Self, token: *mut EfiFileIoToken) -> EfiStatus,
    pub write_ex:
        unsafe extern "efiapi" fn(this: *mut Self, token: *mut EfiFileIoToken) -> EfiStatus,
    pub flush_ex:
        unsafe extern "efiapi" fn(this: *mut Self, token: *mut EfiFileIoToken) -> EfiStatus,
}

pub mod open_mode {
    pub const READ: u64 = 0x0000_0000_0000_0001;
    pub const WRITE: u64 = 0x0000_0000_0000_0002;
    pub const CREATE: u64 = 0x8000_0000_0000_0000;
}

#[repr(C)]
pub struct EfiSimpleFileSystemProtocol {
    pub revision: u64,
    pub open_volume:
        unsafe extern "efiapi" fn(this: *mut Self, root: *mut *mut EfiFileProtocol) -> EfiStatus,
}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_LOADED_IMAGE_PROTOCOL
// ──────────────────────────────────────────────────────────────────────────────

pub static EFI_LOADED_IMAGE_PROTOCOL_REVISION: u32 = 0x1000;

#[repr(C)]
pub struct EfiLoadedImageProtocol {
    pub revision: u32,
    pub parent_handle: EfiHandle,
    pub system_table: *mut EfiSystemTable,

    // ── ソース位置 ────────────────────────────────────────────────────────────
    pub device_handle: EfiHandle,
    pub file_path: *mut EfiDevicePath,
    pub reserved: *mut c_void,

    // ── ロードオプション ──────────────────────────────────────────────────────
    pub load_options_size: u32,
    pub load_options: *mut c_void,

    // ── ロード先 ──────────────────────────────────────────────────────────────
    pub image_base: *mut c_void,
    pub image_size: u64,
    pub image_code_type: EfiMemoryType,
    pub image_data_type: EfiMemoryType,
    pub unload: unsafe extern "efiapi" fn(image_handle: EfiHandle) -> EfiStatus,
}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_GRAPHICS_OUTPUT_PROTOCOL
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Debug)]
pub enum EfiGraphicsPixelFormat {
    PixelRedGreenBlueReserved8BitPerColor,
    PixelBlueGreenRedReserved8BitPerColor,
    PixelBitMask,
    PixelBltOnly,
    PixelFormatMax,
}

#[repr(C)]
pub struct EfiPixelBitmask {
    pub red_mask: u32,
    pub green_mask: u32,
    pub blue_mask: u32,
    pub reserved_mask: u32,
}

#[repr(C)]
pub struct EfiGraphicsOutputModeInformation {
    pub version: u32,
    pub horizontal_resolution: u32,
    pub vertical_resolution: u32,
    pub pixel_format: EfiGraphicsPixelFormat,
    pub pixel_information: EfiPixelBitmask,
    pub pixels_per_scan_line: u32,
}

#[repr(C)]
pub struct EfiGraphicsOutputProtocolMode {
    pub max_mode: u32,
    pub mode: u32,
    pub info: *mut EfiGraphicsOutputModeInformation,
    pub size_of_info: usize,
    pub frame_buffer_base: EfiPhysicalAddress,
    pub frame_buffer_size: usize,
}

/// EFI_GRAPHICS_OUTPUT_BLT_PIXEL
#[repr(C)]
pub struct EfiBltPixel {
    pub blue: u8,
    pub green: u8,
    pub red: u8,
    pub reserved: u8,
}

#[repr(C)]
pub enum EfiBltOperation {
    EfiBltVideoFill,
    EfiBltVideoToBltBuffer,
    EfiBltBufferToVideo,
    EfiBltVideoToVideo,
    EfiGraphicsOutputBltOperationMax,
}

#[repr(C)]
pub struct EfiGraphicsOutputProtocol {
    pub query_mode: unsafe extern "efiapi" fn(
        this: *mut Self,
        mode_number: u32,
        size_of_info: *mut usize,
        info: *mut *mut EfiGraphicsOutputModeInformation,
    ) -> EfiStatus,
    pub set_mode: unsafe extern "efiapi" fn(this: *mut Self, mode_number: u32) -> EfiStatus,
    pub blt: unsafe extern "efiapi" fn(
        this: *mut Self,
        blt_buffer: *mut EfiBltPixel,
        blt_operation: EfiBltOperation,
        source_x: usize,
        source_y: usize,
        destination_x: usize,
        destination_y: usize,
        width: usize,
        height: usize,
        delta: usize,
    ) -> EfiStatus,
    pub mode: *mut EfiGraphicsOutputProtocolMode,
}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_BOOT_SERVICES
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiBootServices {
    pub header: EfiTableHeader,

    // ── Task Priority Services ────────────────────────────────────────────────
    pub raise_tpl: unsafe extern "efiapi" fn(new_tpl: EfiTpl) -> EfiTpl,

    pub restore_tpl: unsafe extern "efiapi" fn(old_tpl: EfiTpl),

    // ── Memory Services ───────────────────────────────────────────────────────
    pub allocate_pages: unsafe extern "efiapi" fn(
        allocate_type: EfiAllocateType,
        memory_type: EfiMemoryType,
        pages: usize,
        memory: *mut EfiPhysicalAddress,
    ) -> EfiStatus,

    pub free_pages:
        unsafe extern "efiapi" fn(memory: EfiPhysicalAddress, pages: usize) -> EfiStatus,

    pub get_memory_map: unsafe extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut EfiMemoryDescriptor,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> EfiStatus,

    pub allocate_pool: unsafe extern "efiapi" fn(
        pool_type: EfiMemoryType,
        size: usize,
        buffer: *mut *mut c_void,
    ) -> EfiStatus,

    pub free_pool: unsafe extern "efiapi" fn(buffer: *mut c_void) -> EfiStatus,

    // ── Event & Timer Services ────────────────────────────────────────────────
    pub create_event: unsafe extern "efiapi" fn(
        r#type: u32,
        notify_tpl: EfiTpl,
        notify_fn: Option<EfiEventNotify>,
        context: *mut c_void,
        event: *mut EfiEvent,
    ) -> EfiStatus,

    pub set_timer: unsafe extern "efiapi" fn(
        event: EfiEvent,
        r#type: EfiTimerDelay,
        trigger_time: u64,
    ) -> EfiStatus,

    pub wait_for_event: unsafe extern "efiapi" fn(
        number_of_events: usize,
        event: *mut EfiEvent,
        index: *mut usize,
    ) -> EfiStatus,

    pub signal_event: unsafe extern "efiapi" fn(event: EfiEvent) -> EfiStatus,

    pub close_event: unsafe extern "efiapi" fn(event: EfiEvent) -> EfiStatus,

    pub check_event: unsafe extern "efiapi" fn(event: EfiEvent) -> EfiStatus,

    // ── Protocol Handler Services ─────────────────────────────────────────────
    pub install_protocol_interface: unsafe extern "efiapi" fn(
        handle: *mut EfiHandle,
        protocol: *const EfiGuid,
        interface_type: EfiInterfaceType,
        interface: *mut c_void,
    ) -> EfiStatus,

    pub reinstall_protocol_interface: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        old_interface: *mut c_void,
        new_interface: *mut c_void,
    ) -> EfiStatus,

    pub uninstall_protocol_interface: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        interface: *mut c_void,
    ) -> EfiStatus,

    pub handle_protocol: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        interface: *mut *mut c_void,
    ) -> EfiStatus,

    pub reserved: *mut c_void,

    pub register_protocol_notify: unsafe extern "efiapi" fn(
        protocol: *const EfiGuid,
        event: EfiEvent,
        registration: *mut *mut c_void,
    ) -> EfiStatus,

    pub locate_handle: unsafe extern "efiapi" fn(
        search_type: EfiLocateSearchType,
        protocol: *const EfiGuid,
        search_key: *mut c_void,
        buffer_size: *mut usize,
        buffer: *mut EfiHandle,
    ) -> EfiStatus,

    pub locate_device_path: unsafe extern "efiapi" fn(
        protocol: *const EfiGuid,
        device_path: *mut *mut EfiDevicePath,
        device: *mut EfiHandle,
    ) -> EfiStatus,

    pub install_configuration_table:
        unsafe extern "efiapi" fn(guid: *const EfiGuid, table: *mut c_void) -> EfiStatus,

    // ── Image Services ────────────────────────────────────────────────────────
    pub load_image: unsafe extern "efiapi" fn(
        boot_policy: bool,
        parent_image_handle: EfiHandle,
        device_path: *mut EfiDevicePath,
        source_buffer: *mut c_void,
        source_size: usize,
        image_handle: *mut EfiHandle,
    ) -> EfiStatus,

    pub start_image: unsafe extern "efiapi" fn(
        image_handle: EfiHandle,
        exit_data_size: *mut usize,
        exit_data: *mut *mut u16,
    ) -> EfiStatus,

    pub exit: unsafe extern "efiapi" fn(
        image_handle: EfiHandle,
        exit_status: EfiStatus,
        exit_data_size: usize,
        exit_data: *mut u16,
    ) -> EfiStatus,

    pub unload_image: unsafe extern "efiapi" fn(image_handle: EfiHandle) -> EfiStatus,

    pub exit_boot_services:
        unsafe extern "efiapi" fn(image_handle: EfiHandle, map_key: usize) -> EfiStatus,

    // ── Miscellaneous Services ────────────────────────────────────────────────
    pub get_next_monotonic_count: unsafe extern "efiapi" fn(count: *mut u64) -> EfiStatus,

    pub stall: unsafe extern "efiapi" fn(microseconds: usize) -> EfiStatus,

    pub set_watchdog_timer: unsafe extern "efiapi" fn(
        timeout: usize,
        watchdog_code: u64,
        data_size: usize,
        watchdog_data: *mut u16,
    ) -> EfiStatus,

    // ── Driver Support Services ───────────────────────────────────────────────
    pub connect_controller: unsafe extern "efiapi" fn(
        controller_handle: EfiHandle,
        driver_image_handle: *mut EfiHandle,
        remaining_device_path: *mut EfiDevicePath,
        recursive: bool,
    ) -> EfiStatus,

    pub disconnect_controller: unsafe extern "efiapi" fn(
        controller_handle: EfiHandle,
        driver_image_handle: EfiHandle,
        child_handle: EfiHandle,
    ) -> EfiStatus,

    // ── Open and Close Protocol Services ─────────────────────────────────────
    pub open_protocol: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        interface: *mut *mut c_void,
        agent_handle: EfiHandle,
        controller_handle: EfiHandle,
        attributes: u32,
    ) -> EfiStatus,

    pub close_protocol: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        agent_handle: EfiHandle,
        controller_handle: EfiHandle,
    ) -> EfiStatus,

    pub open_protocol_information: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        entry_buffer: *mut *mut EfiOpenProtocolInformationEntry,
        entry_count: *mut usize,
    ) -> EfiStatus,

    // ── Library Services ──────────────────────────────────────────────────────
    pub protocols_per_handle: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol_buffer: *mut *mut *mut EfiGuid,
        protocol_buffer_count: *mut usize,
    ) -> EfiStatus,

    pub locate_handle_buffer: unsafe extern "efiapi" fn(
        search_type: EfiLocateSearchType,
        protocol: *const EfiGuid,
        search_key: *mut c_void,
        no_handles: *mut usize,
        buffer: *mut *mut EfiHandle,
    ) -> EfiStatus,

    pub locate_protocol: unsafe extern "efiapi" fn(
        protocol: *const EfiGuid,
        registration: *mut c_void,
        interface: *mut *mut c_void,
    ) -> EfiStatus,

    /// Variadic — represented as a raw pointer; cast to the appropriate
    /// function type when calling.
    pub install_multiple_protocol_interfaces: *const c_void,

    /// Variadic — represented as a raw pointer; cast to the appropriate
    /// function type when calling.
    pub uninstall_multiple_protocol_interfaces: *const c_void,

    // ── 32-bit CRC Services ───────────────────────────────────────────────────
    pub calculate_crc32: unsafe extern "efiapi" fn(
        data: *mut c_void,
        data_size: usize,
        crc32: *mut u32,
    ) -> EfiStatus,

    // ── Miscellaneous Services ────────────────────────────────────────────────
    pub copy_mem:
        unsafe extern "efiapi" fn(destination: *mut c_void, source: *const c_void, length: usize),

    pub set_mem: unsafe extern "efiapi" fn(buffer: *mut c_void, size: usize, value: u8),

    pub create_event_ex: unsafe extern "efiapi" fn(
        r#type: u32,
        notify_tpl: EfiTpl,
        notify_fn: Option<EfiEventNotify>,
        context: *const c_void,
        event_group: *const EfiGuid,
        event: *mut EfiEvent,
    ) -> EfiStatus,
}

// UEFI はシングルスレッド環境であり、実際にはスレッド間共有は発生しない。
// raw pointer フィールドにより Sync が自動実装されないため手動で宣言する。
unsafe impl Sync for EfiBootServices {}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_RUNTIME_SERVICES
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiRuntimeServices {
    pub header: EfiTableHeader,

    // ── Time Services ─────────────────────────────────────────────────────────
    pub get_time: unsafe extern "efiapi" fn(
        time: *mut EfiTime,
        capabilities: *mut EfiTimeCapabilities,
    ) -> EfiStatus,

    pub set_time: unsafe extern "efiapi" fn(time: *const EfiTime) -> EfiStatus,

    pub get_wakeup_time: unsafe extern "efiapi" fn(
        enabled: *mut bool,
        pending: *mut bool,
        time: *mut EfiTime,
    ) -> EfiStatus,

    pub set_wakeup_time: unsafe extern "efiapi" fn(enable: bool, time: *mut EfiTime) -> EfiStatus,

    // ── Virtual Memory Services ───────────────────────────────────────────────
    pub set_virtual_address_map: unsafe extern "efiapi" fn(
        memory_map_size: usize,
        descriptor_size: usize,
        descriptor_version: u32,
        virtual_map: *mut EfiMemoryDescriptor,
    ) -> EfiStatus,

    pub convert_pointer:
        unsafe extern "efiapi" fn(debug_disposition: usize, address: *mut *mut c_void) -> EfiStatus,

    // ── Variable Services ─────────────────────────────────────────────────────
    pub get_variable: unsafe extern "efiapi" fn(
        variable_name: *const u16,
        vendor_guid: *const EfiGuid,
        attributes: *mut u32,
        data_size: *mut usize,
        data: *mut c_void,
    ) -> EfiStatus,

    pub get_next_variable_name: unsafe extern "efiapi" fn(
        variable_name_size: *mut usize,
        variable_name: *mut u16,
        vendor_guid: *const EfiGuid,
    ) -> EfiStatus,

    pub set_variable: unsafe extern "efiapi" fn(
        variable_name: *const u16,
        vendor_guid: *const EfiGuid,
        attributes: u32,
        data_size: usize,
        data: *mut c_void,
    ) -> EfiStatus,

    // ── Miscellaneous Services ────────────────────────────────────────────────
    pub get_next_high_monotonic_count: unsafe extern "efiapi" fn(high_count: *mut u32) -> EfiStatus,

    pub reset_system: unsafe extern "efiapi" fn(
        reset_type: EfiResetType,
        reset_status: EfiStatus,
        data_size: usize,
        reset_data: *mut c_void,
    ),

    // ── UEFI 2.0 Capsule Services ─────────────────────────────────────────────
    pub update_capsule: unsafe extern "efiapi" fn(
        capsule_header_array: *mut *mut EfiCapsuleHeader,
        capsule_count: usize,
        scatter_gather_list: EfiPhysicalAddress,
    ) -> EfiStatus,

    pub query_capsule_capabilities: unsafe extern "efiapi" fn(
        capsule_header_array: *mut *mut EfiCapsuleHeader,
        capsule_count: usize,
        maximum_capsule_size: *mut u64,
        reset_type: *mut EfiResetType,
    ) -> EfiStatus,

    // ── Miscellaneous UEFI 2.0 Service ────────────────────────────────────────
    pub query_variable_info: unsafe extern "efiapi" fn(
        attributes: u32,
        maximum_variable_storage_size: *mut u64,
        remaining_variable_storage_size: *mut u64,
        maximum_variable_size: *mut u64,
    ) -> EfiStatus,
}

// ──────────────────────────────────────────────────────────────────────────────
// EFI_SYSTEM_TABLE
// ──────────────────────────────────────────────────────────────────────────────

#[repr(C)]
pub struct EfiSystemTable {
    pub header: EfiTableHeader,
    pub firmware_vendor: *const u16,
    pub firmware_revision: u32,
    pub console_in_handle: EfiHandle,
    pub con_in: *mut EfiSimpleTextInputProtocol,
    pub console_out_handle: EfiHandle,
    pub con_out: *mut EfiSimpleTextOutputProtocol,
    pub standard_error_handle: EfiHandle,
    pub std_err: *mut EfiSimpleTextOutputProtocol,
    pub runtime_services: *mut EfiRuntimeServices,
    pub boot_services: *mut EfiBootServices,
    pub number_of_table_entries: usize,
    pub configuration_table: *mut EfiConfigurationTable,
}
