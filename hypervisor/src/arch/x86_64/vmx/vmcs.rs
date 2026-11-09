//! VMCS のフィールド定義と VMX 命令のラッパ（Intel SDM Vol.3 Appendix B）

use core::arch::asm;

/// VMX 命令が失敗したときの情報
// フィールドは panic 時の Debug 出力で使う
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum VmxError {
    /// VMfailInvalid（カレント VMCS がない）
    FailInvalid,
    /// VMfailValid（VM-instruction error field の値）
    FailValid(u64),
}

#[allow(dead_code)]
pub mod field {
    // 16bit control
    pub const VPID: u64 = 0x0000;
    // 16bit guest state
    pub const GUEST_ES_SELECTOR: u64 = 0x0800;
    pub const GUEST_CS_SELECTOR: u64 = 0x0802;
    pub const GUEST_SS_SELECTOR: u64 = 0x0804;
    pub const GUEST_DS_SELECTOR: u64 = 0x0806;
    pub const GUEST_FS_SELECTOR: u64 = 0x0808;
    pub const GUEST_GS_SELECTOR: u64 = 0x080a;
    pub const GUEST_LDTR_SELECTOR: u64 = 0x080c;
    pub const GUEST_TR_SELECTOR: u64 = 0x080e;
    // 16bit host state
    pub const HOST_ES_SELECTOR: u64 = 0x0c00;
    pub const HOST_CS_SELECTOR: u64 = 0x0c02;
    pub const HOST_SS_SELECTOR: u64 = 0x0c04;
    pub const HOST_DS_SELECTOR: u64 = 0x0c06;
    pub const HOST_FS_SELECTOR: u64 = 0x0c08;
    pub const HOST_GS_SELECTOR: u64 = 0x0c0a;
    pub const HOST_TR_SELECTOR: u64 = 0x0c0c;
    // 64bit control
    pub const MSR_BITMAP: u64 = 0x2004;
    pub const EPT_POINTER: u64 = 0x201a;
    // 64bit read-only
    pub const GUEST_PHYSICAL_ADDRESS: u64 = 0x2400;
    // 64bit guest state
    pub const VMCS_LINK_POINTER: u64 = 0x2800;
    pub const GUEST_IA32_DEBUGCTL: u64 = 0x2802;
    pub const GUEST_IA32_PAT: u64 = 0x2804;
    pub const GUEST_IA32_EFER: u64 = 0x2806;
    // 64bit host state
    pub const HOST_IA32_PAT: u64 = 0x2c00;
    pub const HOST_IA32_EFER: u64 = 0x2c02;
    // 32bit control
    pub const PIN_BASED_VM_EXEC_CONTROL: u64 = 0x4000;
    pub const CPU_BASED_VM_EXEC_CONTROL: u64 = 0x4002;
    pub const EXCEPTION_BITMAP: u64 = 0x4004;
    pub const CR3_TARGET_COUNT: u64 = 0x400a;
    pub const VM_EXIT_CONTROLS: u64 = 0x400c;
    pub const VM_EXIT_MSR_STORE_COUNT: u64 = 0x400e;
    pub const VM_EXIT_MSR_LOAD_COUNT: u64 = 0x4010;
    pub const VM_ENTRY_CONTROLS: u64 = 0x4012;
    pub const VM_ENTRY_MSR_LOAD_COUNT: u64 = 0x4014;
    pub const VM_ENTRY_INTR_INFO: u64 = 0x4016;
    pub const VM_ENTRY_EXCEPTION_ERROR_CODE: u64 = 0x4018;
    pub const VM_ENTRY_INSTRUCTION_LEN: u64 = 0x401a;
    pub const SECONDARY_VM_EXEC_CONTROL: u64 = 0x401e;
    // 32bit read-only
    pub const VM_INSTRUCTION_ERROR: u64 = 0x4400;
    pub const VM_EXIT_REASON: u64 = 0x4402;
    pub const VM_EXIT_INTR_INFO: u64 = 0x4404;
    pub const VM_EXIT_INTR_ERROR_CODE: u64 = 0x4406;
    pub const IDT_VECTORING_INFO: u64 = 0x4408;
    pub const IDT_VECTORING_ERROR_CODE: u64 = 0x440a;
    pub const VM_EXIT_INSTRUCTION_LEN: u64 = 0x440c;
    pub const VM_EXIT_INSTRUCTION_INFO: u64 = 0x440e;
    // 32bit guest state
    pub const GUEST_ES_LIMIT: u64 = 0x4800;
    pub const GUEST_CS_LIMIT: u64 = 0x4802;
    pub const GUEST_SS_LIMIT: u64 = 0x4804;
    pub const GUEST_DS_LIMIT: u64 = 0x4806;
    pub const GUEST_FS_LIMIT: u64 = 0x4808;
    pub const GUEST_GS_LIMIT: u64 = 0x480a;
    pub const GUEST_LDTR_LIMIT: u64 = 0x480c;
    pub const GUEST_TR_LIMIT: u64 = 0x480e;
    pub const GUEST_GDTR_LIMIT: u64 = 0x4810;
    pub const GUEST_IDTR_LIMIT: u64 = 0x4812;
    pub const GUEST_ES_AR_BYTES: u64 = 0x4814;
    pub const GUEST_CS_AR_BYTES: u64 = 0x4816;
    pub const GUEST_SS_AR_BYTES: u64 = 0x4818;
    pub const GUEST_DS_AR_BYTES: u64 = 0x481a;
    pub const GUEST_FS_AR_BYTES: u64 = 0x481c;
    pub const GUEST_GS_AR_BYTES: u64 = 0x481e;
    pub const GUEST_LDTR_AR_BYTES: u64 = 0x4820;
    pub const GUEST_TR_AR_BYTES: u64 = 0x4822;
    pub const GUEST_INTERRUPTIBILITY_INFO: u64 = 0x4824;
    pub const GUEST_ACTIVITY_STATE: u64 = 0x4826;
    pub const GUEST_SYSENTER_CS: u64 = 0x482a;
    // 32bit host state
    pub const HOST_IA32_SYSENTER_CS: u64 = 0x4c00;
    // natural-width control
    pub const CR0_GUEST_HOST_MASK: u64 = 0x6000;
    pub const CR4_GUEST_HOST_MASK: u64 = 0x6002;
    pub const CR0_READ_SHADOW: u64 = 0x6004;
    pub const CR4_READ_SHADOW: u64 = 0x6006;
    // natural-width read-only
    pub const EXIT_QUALIFICATION: u64 = 0x6400;
    pub const GUEST_LINEAR_ADDRESS: u64 = 0x640a;
    // natural-width guest state
    pub const GUEST_CR0: u64 = 0x6800;
    pub const GUEST_CR3: u64 = 0x6802;
    pub const GUEST_CR4: u64 = 0x6804;
    pub const GUEST_ES_BASE: u64 = 0x6806;
    pub const GUEST_CS_BASE: u64 = 0x6808;
    pub const GUEST_SS_BASE: u64 = 0x680a;
    pub const GUEST_DS_BASE: u64 = 0x680c;
    pub const GUEST_FS_BASE: u64 = 0x680e;
    pub const GUEST_GS_BASE: u64 = 0x6810;
    pub const GUEST_LDTR_BASE: u64 = 0x6812;
    pub const GUEST_TR_BASE: u64 = 0x6814;
    pub const GUEST_GDTR_BASE: u64 = 0x6816;
    pub const GUEST_IDTR_BASE: u64 = 0x6818;
    pub const GUEST_DR7: u64 = 0x681a;
    pub const GUEST_RSP: u64 = 0x681c;
    pub const GUEST_RIP: u64 = 0x681e;
    pub const GUEST_RFLAGS: u64 = 0x6820;
    pub const GUEST_PENDING_DBG_EXCEPTIONS: u64 = 0x6822;
    pub const GUEST_SYSENTER_ESP: u64 = 0x6824;
    pub const GUEST_SYSENTER_EIP: u64 = 0x6826;
    // natural-width host state
    pub const HOST_CR0: u64 = 0x6c00;
    pub const HOST_CR3: u64 = 0x6c02;
    pub const HOST_CR4: u64 = 0x6c04;
    pub const HOST_FS_BASE: u64 = 0x6c06;
    pub const HOST_GS_BASE: u64 = 0x6c08;
    pub const HOST_TR_BASE: u64 = 0x6c0a;
    pub const HOST_GDTR_BASE: u64 = 0x6c0c;
    pub const HOST_IDTR_BASE: u64 = 0x6c0e;
    pub const HOST_IA32_SYSENTER_ESP: u64 = 0x6c10;
    pub const HOST_IA32_SYSENTER_EIP: u64 = 0x6c12;
    pub const HOST_RSP: u64 = 0x6c14;
    pub const HOST_RIP: u64 = 0x6c16;
}

/// VMX 命令実行後の RFLAGS から成否を判定する
fn check_vmx_result(rflags: u64) -> Result<(), VmxError> {
    const CF: u64 = 1 << 0;
    const ZF: u64 = 1 << 6;
    if rflags & CF != 0 {
        Err(VmxError::FailInvalid)
    } else if rflags & ZF != 0 {
        Err(VmxError::FailValid(
            vmread(field::VM_INSTRUCTION_ERROR).unwrap_or(0),
        ))
    } else {
        Ok(())
    }
}

/// # Safety
/// `vmxon_region` は IA32_VMX_BASIC のリビジョン ID を書いた 4KiB アラインの物理ページであること
pub unsafe fn vmxon(vmxon_region: u64) -> Result<(), VmxError> {
    let rflags: u64;
    unsafe {
        asm!("vmxon [{}]", "pushfq", "pop {}", in(reg) &vmxon_region, lateout(reg) rflags);
    }
    check_vmx_result(rflags)
}

/// # Safety
/// `vmcs` はリビジョン ID を書いた 4KiB アラインの物理ページであること
pub unsafe fn vmclear(vmcs: u64) -> Result<(), VmxError> {
    let rflags: u64;
    unsafe {
        asm!("vmclear [{}]", "pushfq", "pop {}", in(reg) &vmcs, lateout(reg) rflags);
    }
    check_vmx_result(rflags)
}

/// # Safety
/// `vmcs` は VMCLEAR 済みの VMCS 領域の物理アドレスであること
pub unsafe fn vmptrld(vmcs: u64) -> Result<(), VmxError> {
    let rflags: u64;
    unsafe {
        asm!("vmptrld [{}]", "pushfq", "pop {}", in(reg) &vmcs, lateout(reg) rflags);
    }
    check_vmx_result(rflags)
}

pub fn vmread(field: u64) -> Result<u64, VmxError> {
    let value: u64;
    let rflags: u64;
    unsafe {
        asm!("vmread {}, {}", "pushfq", "pop {}", out(reg) value, in(reg) field, lateout(reg) rflags);
    }
    // vmread の失敗時に VM_INSTRUCTION_ERROR を読みに行くと再帰するので、ここでは直接判定する
    if rflags & 1 != 0 || rflags & (1 << 6) != 0 {
        return Err(VmxError::FailInvalid);
    }
    Ok(value)
}

pub fn vmwrite(field: u64, value: u64) -> Result<(), VmxError> {
    let rflags: u64;
    unsafe {
        asm!("vmwrite {}, {}", "pushfq", "pop {}", in(reg) field, in(reg) value, lateout(reg) rflags);
    }
    check_vmx_result(rflags)
}

/// カレント VMCS の読み出し。exit 処理中など、カレント VMCS があることが確実な場面で使う
pub fn read(field: u64) -> u64 {
    vmread(field).unwrap_or_else(|e| panic!("vmread({:#x}) failed: {:?}", field, e))
}

/// カレント VMCS への書き込み。失敗は設定ミスなので panic する
pub fn write(field: u64, value: u64) {
    vmwrite(field, value)
        .unwrap_or_else(|e| panic!("vmwrite({:#x}, {:#x}) failed: {:?}", field, value, e))
}
