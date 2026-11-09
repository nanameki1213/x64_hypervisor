//! MTRR からメモリタイプを求める。
//!
//! EPT を有効にすると、ゲストのアクセスには MTRR ではなく EPT のメモリタイプが使われる。
//! そのため EPT のリーフには、物理 MTRR が示すタイプをそのまま設定する。

use super::cpu;

pub const UC: u8 = 0;
pub const WC: u8 = 1;
pub const WT: u8 = 4;
pub const WP: u8 = 5;
pub const WB: u8 = 6;

const IA32_MTRRCAP: u32 = 0xfe;
const IA32_MTRR_DEF_TYPE: u32 = 0x2ff;
const IA32_MTRR_PHYSBASE0: u32 = 0x200;
const IA32_MTRR_FIX64K_00000: u32 = 0x250;
const IA32_MTRR_FIX16K_80000: u32 = 0x258;
const IA32_MTRR_FIX4K_C0000: u32 = 0x268;

const FIXED_RANGE_END: u64 = 0x10_0000;
const PAGE_SIZE: u64 = 0x1000;

#[derive(Clone, Copy)]
struct VariableRange {
    base: u64,
    mask: u64,
    mem_type: u8,
}

pub struct Mtrr {
    enabled: bool,
    default_type: u8,
    /// 0〜1MiB の 4KiB ページごとのタイプ（固定範囲 MTRR が無効なら None）
    fixed: Option<[u8; (FIXED_RANGE_END / PAGE_SIZE) as usize]>,
    variable: [Option<VariableRange>; 32],
}

impl Mtrr {
    pub fn read() -> Self {
        const CAP_FIX: u64 = 1 << 8;
        const DEF_FE: u64 = 1 << 10;
        const DEF_E: u64 = 1 << 11;
        const PHYSMASK_VALID: u64 = 1 << 11;

        let cap = cpu::rdmsr(IA32_MTRRCAP);
        let def = cpu::rdmsr(IA32_MTRR_DEF_TYPE);
        let enabled = def & DEF_E != 0;

        let fixed = (enabled && cap & CAP_FIX != 0 && def & DEF_FE != 0).then(|| {
            let mut types = [0u8; (FIXED_RANGE_END / PAGE_SIZE) as usize];
            // (MSR, 1 エントリのサイズ, MSR の数)
            let groups = [
                (IA32_MTRR_FIX64K_00000, 0x10000u64, 1u32),
                (IA32_MTRR_FIX16K_80000, 0x4000, 2),
                (IA32_MTRR_FIX4K_C0000, 0x1000, 8),
            ];
            let mut addr = 0u64;
            for (first_msr, entry_size, count) in groups {
                for msr in first_msr..first_msr + count {
                    let value = cpu::rdmsr(msr);
                    for byte in 0..8 {
                        let mem_type = (value >> (byte * 8)) as u8;
                        for page in 0..entry_size / PAGE_SIZE {
                            types[(addr / PAGE_SIZE + page) as usize] = mem_type;
                        }
                        addr += entry_size;
                    }
                }
            }
            types
        });

        let phys_bits = cpu::cpuid(0x8000_0008, 0).eax & 0xff;
        let addr_mask = ((1u64 << phys_bits) - 1) & !(PAGE_SIZE - 1);
        let mut variable = [None; 32];
        for (i, slot) in variable.iter_mut().enumerate().take((cap & 0xff) as usize) {
            let base = cpu::rdmsr(IA32_MTRR_PHYSBASE0 + 2 * i as u32);
            let mask = cpu::rdmsr(IA32_MTRR_PHYSBASE0 + 2 * i as u32 + 1);
            if mask & PHYSMASK_VALID != 0 {
                *slot = Some(VariableRange {
                    base: base & addr_mask,
                    mask: mask & addr_mask,
                    mem_type: base as u8,
                });
            }
        }

        Mtrr {
            enabled,
            default_type: (def & 0xff) as u8,
            fixed,
            variable,
        }
    }

    /// `size`（2 のべき乗）にアラインされた `[start, start + size)` 全体のメモリタイプ。
    /// 範囲内でタイプが一様でなければ None を返すので、呼び出し側で小さいページに分割すること
    pub fn memory_type(&self, start: u64, size: u64) -> Option<u8> {
        debug_assert!(size.is_power_of_two() && start.is_multiple_of(size) && size >= PAGE_SIZE);
        if !self.enabled {
            return Some(UC);
        }

        if start < FIXED_RANGE_END
            && let Some(fixed) = &self.fixed
        {
            if start + size > FIXED_RANGE_END {
                return None;
            }
            let first = (start / PAGE_SIZE) as usize;
            let pages = &fixed[first..first + (size / PAGE_SIZE) as usize];
            return pages.iter().all(|&t| t == pages[0]).then_some(pages[0]);
        }

        let mut matched: Option<u8> = None;
        for range in self.variable.iter().flatten() {
            // 範囲のサイズ以上のビットで一致しなければ、範囲内のどのアドレスとも一致しない
            let high = range.mask & !(size - 1);
            if start & high != range.base & high {
                continue;
            }
            // マスクが範囲のサイズより細かいビットを見るなら、範囲内で一致・不一致が混ざる
            if range.mask & (size - 1) != 0 {
                return None;
            }
            matched = Some(match (matched, range.mem_type) {
                (None, t) => t,
                (Some(a), b) if a == b => a,
                // 重なった場合の優先順位（SDM 12.11.4.1）
                (Some(UC), _) | (_, UC) => UC,
                (Some(WT), WB) | (Some(WB), WT) => WT,
                // それ以外の重なりは未定義なので、安全側に倒す
                _ => UC,
            });
        }
        Some(matched.unwrap_or(self.default_type))
    }
}

#[allow(dead_code)]
pub fn type_name(t: u8) -> &'static str {
    match t {
        UC => "UC",
        WC => "WC",
        WT => "WT",
        WP => "WP",
        WB => "WB",
        _ => "??",
    }
}
