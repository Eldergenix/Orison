//! Hand-rolled x86_64 machine-code emitter (bootstrap subset).
//!
//! This module produces real Linux x86_64 instruction bytes — no LLVM, no
//! Cranelift, no `iced-x86`. It implements just enough of Intel SDM
//! Volume 2 to cover the instructions Orison's bootstrap backend currently
//! needs: register-to-register data movement and arithmetic, push/pop, a
//! handful of conditional branches, syscall, ret, and nop.
//!
//! References (Intel 64 and IA-32 Architectures Software Developer's
//! Manual, Volume 2, December 2024 revision, abbreviated "SDM"):
//!   * REX prefix encoding ............................. SDM Vol 2A §2.2.1
//!   * ModR/M byte ..................................... SDM Vol 2A §2.1.3
//!   * MOV r64, imm64 (B8+rd io) ....................... SDM Vol 2B p. 4-35
//!   * MOV r/m64, r64 (89 /r) .......................... SDM Vol 2B p. 4-35
//!   * ADD r/m64, r64 (01 /r) .......................... SDM Vol 2A p. 3-29
//!   * SUB r/m64, r64 (29 /r) .......................... SDM Vol 2B p. 4-510
//!   * IMUL r64, r/m64 (0F AF /r) ...................... SDM Vol 2A p. 3-528
//!   * PUSH r64 (50+rd) / POP r64 (58+rd) .............. SDM Vol 2B p. 4-279
//!   * CMP r/m64, r64 (39 /r) .......................... SDM Vol 2A p. 3-159
//!   * Jcc rel32 (0F 8x cd) ............................ SDM Vol 2A p. 3-587
//!   * JMP rel32 (E9 cd) ............................... SDM Vol 2A p. 3-595
//!   * RET (C3) ........................................ SDM Vol 2B p. 4-441
//!   * SYSCALL (0F 05) ................................. SDM Vol 2B p. 4-685
//!   * NOP (90) ........................................ SDM Vol 2B p. 4-184
//!
//! ## Determinism guarantees
//!
//! `encode` is a pure function: the same input slice always yields the same
//! output byte vector, byte for byte. `encode_with_report` additionally
//! hashes the emitted bytes with FNV-1a (64-bit) so downstream tooling can
//! cheaply detect drift without re-encoding.
//!
//! ## Safety
//!
//! No unsafe blocks. No unhandled-error sites. No production-source
//! guardrail violations (see CONTRIBUTING.md). Every input variant is
//! exhaustively matched in `encode_instr`.

use serde::{Deserialize, Serialize};

/// JSON Schema id reported by [`X64Report`].
pub const X64_REPORT_SCHEMA: &str = "ori.x64_codegen_report.v1";

/// FNV-1a 64-bit constants (`offset_basis` and `prime`) from the original
/// reference implementation by Fowler/Noll/Vo. Used for the deterministic
/// fingerprint embedded in [`X64Report::hash`].
const FNV1A_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV1A_PRIME: u64 = 0x0000_0100_0000_01b3;

/// The 16 general-purpose 64-bit registers of x86_64, in the canonical
/// ordering used throughout SDM Vol 2 tables (RAX=0 … R15=15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reg {
    Rax,
    Rcx,
    Rdx,
    Rbx,
    Rsp,
    Rbp,
    Rsi,
    Rdi,
    R8,
    R9,
    R10,
    R11,
    R12,
    R13,
    R14,
    R15,
}

impl Reg {
    /// Canonical 4-bit register index. Bit 3 selects the extended (`r8`-`r15`)
    /// half and is carried into the appropriate REX bit (`B` or `R`); the
    /// low 3 bits go directly into the ModR/M `reg` or `rm` field.
    #[inline]
    pub fn index(self) -> u8 {
        match self {
            Reg::Rax => 0,
            Reg::Rcx => 1,
            Reg::Rdx => 2,
            Reg::Rbx => 3,
            Reg::Rsp => 4,
            Reg::Rbp => 5,
            Reg::Rsi => 6,
            Reg::Rdi => 7,
            Reg::R8 => 8,
            Reg::R9 => 9,
            Reg::R10 => 10,
            Reg::R11 => 11,
            Reg::R12 => 12,
            Reg::R13 => 13,
            Reg::R14 => 14,
            Reg::R15 => 15,
        }
    }

    /// Low 3 bits of the register encoding (the value that goes into the
    /// ModR/M `reg` or `rm` field, or the opcode for `B8+rd` / `50+rd`).
    #[inline]
    pub fn low3(self) -> u8 {
        self.index() & 0b111
    }

    /// Whether this register requires the extended (`r8`-`r15`) bit to be
    /// set in the appropriate REX field.
    #[inline]
    pub fn is_extended(self) -> bool {
        self.index() >= 8
    }
}

/// Instruction shapes supported by the bootstrap emitter. Every variant
/// corresponds to a single, fixed-length x86_64 encoding produced by
/// [`encode_instr`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum X64Instr {
    /// `MOV r64, imm64` — `REX.W [REX.B] B8+rd io`. 10 or 11 bytes.
    MovRegImm64(Reg, i64),
    /// `MOV r/m64, r64` — `REX.W [REX.RB] 89 /r`. 3 bytes.
    /// Tuple order is `(dst, src)`; the source is encoded in ModR/M.reg.
    MovRegReg(Reg, Reg),
    /// `ADD r/m64, r64` — `REX.W [REX.RB] 01 /r`. 3 bytes. `(dst, src)`.
    AddRegReg(Reg, Reg),
    /// `SUB r/m64, r64` — `REX.W [REX.RB] 29 /r`. 3 bytes. `(dst, src)`.
    SubRegReg(Reg, Reg),
    /// `IMUL r64, r/m64` — `REX.W [REX.RB] 0F AF /r`. 4 bytes. `(dst, src)`.
    /// Note: in IMUL the destination is the ModR/M.reg field.
    ImulRegReg(Reg, Reg),
    /// `PUSH r64` — `[REX.B] 50+rd`. 1 or 2 bytes (no REX.W; 64-bit default).
    PushReg(Reg),
    /// `POP r64` — `[REX.B] 58+rd`. 1 or 2 bytes (no REX.W; 64-bit default).
    PopReg(Reg),
    /// `RET` — `C3`. 1 byte. Near return.
    Ret,
    /// `SYSCALL` — `0F 05`. 2 bytes. Linux x86_64 syscall entry.
    Syscall,
    /// `JMP rel32` — `E9 cd`. 5 bytes. Relative to the *next* instruction.
    JmpRel32(i32),
    /// `CMP r/m64, r64` — `REX.W [REX.RB] 39 /r`. 3 bytes. `(lhs, rhs)`.
    CmpRegReg(Reg, Reg),
    /// `JE rel32` — `0F 84 cd`. 6 bytes.
    Je(i32),
    /// `JNE rel32` — `0F 85 cd`. 6 bytes.
    Jne(i32),
    /// `JL rel32` (signed less-than) — `0F 8C cd`. 6 bytes.
    Jl(i32),
    /// `JG rel32` (signed greater-than) — `0F 8F cd`. 6 bytes.
    Jg(i32),
    /// `NOP` — `90`. 1 byte.
    Nop,
}

/// Structured report returned by [`encode_with_report`].
///
/// Matches `schemas/x64-codegen-report.schema.json`. The `hash` field is a
/// deterministic FNV-1a 64-bit digest of the encoded bytes, prefixed with
/// `fnv1a:` and rendered as 16 lowercase hex characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct X64Report {
    pub schema: &'static str,
    pub byte_count: usize,
    pub instruction_count: usize,
    pub hash: String,
}

/// Encode an entire instruction sequence into a freshly-allocated byte
/// buffer. Pure function: deterministic given the same input.
pub fn encode(prog: &[X64Instr]) -> Vec<u8> {
    // Pre-size for the common case where instructions average ~4 bytes.
    let mut out = Vec::with_capacity(prog.len() * 4);
    for instr in prog {
        encode_instr(*instr, &mut out);
    }
    out
}

/// Encode `prog` and additionally return a [`X64Report`] containing the
/// emitted byte count, instruction count, and an FNV-1a fingerprint of the
/// bytes. The byte vector is identical to what [`encode`] would return for
/// the same input.
pub fn encode_with_report(prog: &[X64Instr]) -> (Vec<u8>, X64Report) {
    let bytes = encode(prog);
    let hash = format!("fnv1a:{:016x}", fnv1a_64(&bytes));
    let report = X64Report {
        schema: X64_REPORT_SCHEMA,
        byte_count: bytes.len(),
        instruction_count: prog.len(),
        hash,
    };
    (bytes, report)
}

/// Build the canonical "hello world" instruction sequence: write the string
/// `"Hello from Orison\n"` (18 bytes) to stdout via the Linux `write(2)`
/// syscall, then call `exit(0)`. The string data and its load address are
/// intentionally *not* part of the emitted code; this routine emits only the
/// instruction stream that a linker / loader would later resolve. The
/// placeholder address `0x0000_0000_0040_0000` matches the conventional
/// base address of a statically linked Linux x86_64 binary so the bytes are
/// reproducible across runs.
pub fn emit_hello_world_program() -> Vec<X64Instr> {
    // System V AMD64 / Linux syscall ABI:
    //   rax = syscall number     rdi = arg0     rsi = arg1     rdx = arg2
    // write(int fd, const void *buf, size_t count) is syscall 1.
    // exit(int status)                              is syscall 60.
    const HELLO_TEXT_PLACEHOLDER_ADDR: i64 = 0x0000_0000_0040_0000;
    const HELLO_TEXT_LEN: i64 = 18; // "Hello from Orison\n"
    const SYS_WRITE: i64 = 1;
    const SYS_EXIT: i64 = 60;
    const STDOUT_FD: i64 = 1;

    vec![
        X64Instr::MovRegImm64(Reg::Rax, SYS_WRITE),
        X64Instr::MovRegImm64(Reg::Rdi, STDOUT_FD),
        X64Instr::MovRegImm64(Reg::Rsi, HELLO_TEXT_PLACEHOLDER_ADDR),
        X64Instr::MovRegImm64(Reg::Rdx, HELLO_TEXT_LEN),
        X64Instr::Syscall,
        X64Instr::MovRegImm64(Reg::Rax, SYS_EXIT),
        X64Instr::MovRegImm64(Reg::Rdi, 0),
        X64Instr::Syscall,
    ]
}

// ---------------------------------------------------------------------
// Encoding helpers
// ---------------------------------------------------------------------

/// Build a REX prefix byte. `w`, `r`, `x`, `b` are 0/1 flags.
///
/// Layout (SDM Vol 2A §2.2.1.2):
///   `0100 WRXB`
#[inline]
fn rex(w: u8, r: u8, x: u8, b: u8) -> u8 {
    0x40 | ((w & 1) << 3) | ((r & 1) << 2) | ((x & 1) << 1) | (b & 1)
}

/// Build a ModR/M byte (SDM Vol 2A §2.1.3).
///
/// `mod_` is the addressing mode (0..=3); `reg` and `rm` are the low 3 bits
/// of the corresponding register encodings (any extended bit must already
/// have been forwarded into the REX prefix by the caller).
#[inline]
fn modrm(mod_: u8, reg: u8, rm: u8) -> u8 {
    ((mod_ & 0b11) << 6) | ((reg & 0b111) << 3) | (rm & 0b111)
}

/// Encode a single instruction onto the tail of `out`.
fn encode_instr(instr: X64Instr, out: &mut Vec<u8>) {
    match instr {
        X64Instr::MovRegImm64(reg, imm) => emit_mov_reg_imm64(reg, imm, out),
        X64Instr::MovRegReg(dst, src) => emit_rm_r_w(0x89, dst, src, out),
        X64Instr::AddRegReg(dst, src) => emit_rm_r_w(0x01, dst, src, out),
        X64Instr::SubRegReg(dst, src) => emit_rm_r_w(0x29, dst, src, out),
        X64Instr::CmpRegReg(lhs, rhs) => emit_rm_r_w(0x39, lhs, rhs, out),
        X64Instr::ImulRegReg(dst, src) => emit_imul(dst, src, out),
        X64Instr::PushReg(reg) => emit_push_pop(0x50, reg, out),
        X64Instr::PopReg(reg) => emit_push_pop(0x58, reg, out),
        X64Instr::Ret => out.push(0xC3),
        X64Instr::Syscall => out.extend_from_slice(&[0x0F, 0x05]),
        X64Instr::Nop => out.push(0x90),
        X64Instr::JmpRel32(rel) => {
            out.push(0xE9);
            out.extend_from_slice(&rel.to_le_bytes());
        }
        X64Instr::Je(rel) => emit_jcc(0x84, rel, out),
        X64Instr::Jne(rel) => emit_jcc(0x85, rel, out),
        X64Instr::Jl(rel) => emit_jcc(0x8C, rel, out),
        X64Instr::Jg(rel) => emit_jcc(0x8F, rel, out),
    }
}

/// `MOV r64, imm64` — `REX.W [REX.B] B8+rd io`.
fn emit_mov_reg_imm64(reg: Reg, imm: i64, out: &mut Vec<u8>) {
    let b = if reg.is_extended() { 1 } else { 0 };
    out.push(rex(1, 0, 0, b));
    out.push(0xB8 + reg.low3());
    out.extend_from_slice(&imm.to_le_bytes());
}

/// Shared encoder for `<opcode> r/m64, r64` style instructions (MOV/ADD/
/// SUB/CMP) where the source occupies ModR/M.reg and the destination
/// occupies ModR/M.rm. REX.W is always set; REX.R follows the source's
/// extended bit, REX.B the destination's.
fn emit_rm_r_w(opcode: u8, rm_reg: Reg, reg_reg: Reg, out: &mut Vec<u8>) {
    let r = if reg_reg.is_extended() { 1 } else { 0 };
    let b = if rm_reg.is_extended() { 1 } else { 0 };
    out.push(rex(1, r, 0, b));
    out.push(opcode);
    out.push(modrm(0b11, reg_reg.low3(), rm_reg.low3()));
}

/// `IMUL r64, r/m64` — `REX.W [REX.RB] 0F AF /r`. Destination is encoded
/// in ModR/M.reg (extends via REX.R); source is in ModR/M.rm (extends via
/// REX.B).
fn emit_imul(dst: Reg, src: Reg, out: &mut Vec<u8>) {
    let r = if dst.is_extended() { 1 } else { 0 };
    let b = if src.is_extended() { 1 } else { 0 };
    out.push(rex(1, r, 0, b));
    out.push(0x0F);
    out.push(0xAF);
    out.push(modrm(0b11, dst.low3(), src.low3()));
}

/// `PUSH r64` (`50+rd`) and `POP r64` (`58+rd`) share an identical wrapper:
/// no REX.W (operand size is 64-bit by default in long mode), REX.B if the
/// register is `r8`-`r15`. The `base` argument is `0x50` or `0x58`.
fn emit_push_pop(base: u8, reg: Reg, out: &mut Vec<u8>) {
    if reg.is_extended() {
        out.push(rex(0, 0, 0, 1));
    }
    out.push(base + reg.low3());
}

/// `Jcc rel32` — `0F <op> cd`. `op` is the second opcode byte (e.g. `0x84`
/// for JE).
fn emit_jcc(op: u8, rel: i32, out: &mut Vec<u8>) {
    out.push(0x0F);
    out.push(op);
    out.extend_from_slice(&rel.to_le_bytes());
}

/// FNV-1a 64-bit hash of `bytes` (Fowler/Noll/Vo, 1991).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV1A_OFFSET_BASIS;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV1A_PRIME);
    }
    hash
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- helpers ----------------------------------------------------

    /// Encode a single instruction and assert byte equality with a
    /// pre-computed expected sequence.
    fn assert_encodes_to(instr: X64Instr, expected: &[u8]) {
        let bytes = encode(&[instr]);
        assert_eq!(
            bytes.as_slice(),
            expected,
            "instr={:?} encoded={:02X?} expected={:02X?}",
            instr,
            bytes,
            expected
        );
    }

    // ---- 1. Each instruction encoding has a known-good byte sequence ----

    #[test]
    fn encodes_ret_as_single_c3_byte() {
        assert_encodes_to(X64Instr::Ret, &[0xC3]);
    }

    #[test]
    fn encodes_nop_as_single_90_byte() {
        assert_encodes_to(X64Instr::Nop, &[0x90]);
    }

    #[test]
    fn encodes_syscall_as_0f_05() {
        assert_encodes_to(X64Instr::Syscall, &[0x0F, 0x05]);
    }

    #[test]
    fn encodes_mov_rax_imm64_one() {
        // mov rax, 1  →  48 B8 01 00 00 00 00 00 00 00
        assert_encodes_to(
            X64Instr::MovRegImm64(Reg::Rax, 1),
            &[0x48, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn encodes_mov_rdi_imm64_zero() {
        // mov rdi, 0  →  48 BF 00 00 00 00 00 00 00 00
        assert_encodes_to(
            X64Instr::MovRegImm64(Reg::Rdi, 0),
            &[0x48, 0xBF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn encodes_mov_r8_imm64_negative_one_little_endian() {
        // mov r8, -1  →  49 B8 FF FF FF FF FF FF FF FF
        assert_encodes_to(
            X64Instr::MovRegImm64(Reg::R8, -1),
            &[0x49, 0xB8, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        );
    }

    #[test]
    fn encodes_mov_r15_imm64_uses_rex_b_and_low3_rd() {
        // mov r15, 0x1122334455667788
        //   REX = 49 (W=1,B=1)
        //   opcode = B8 + (15 & 7) = BF
        //   imm64 little-endian
        assert_encodes_to(
            X64Instr::MovRegImm64(Reg::R15, 0x1122_3344_5566_7788),
            &[
                0x49, 0xBF, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22, 0x11,
            ],
        );
    }

    #[test]
    fn encodes_mov_rax_rcx_as_rex_w_89_c8() {
        // mov rax, rcx → 48 89 C8  (src=rcx=1 in reg, dst=rax=0 in rm,
        // ModR/M = 11_001_000 = 0xC8)
        assert_encodes_to(
            X64Instr::MovRegReg(Reg::Rax, Reg::Rcx),
            &[0x48, 0x89, 0xC8],
        );
    }

    #[test]
    fn encodes_add_rax_rcx_as_48_01_c8() {
        // add rax, rcx → 48 01 C8
        assert_encodes_to(
            X64Instr::AddRegReg(Reg::Rax, Reg::Rcx),
            &[0x48, 0x01, 0xC8],
        );
    }

    #[test]
    fn encodes_sub_rbx_rdx_as_48_29_d3() {
        // sub rbx, rdx → 48 29 D3   (rdx=2 in reg, rbx=3 in rm,
        // ModR/M = 11_010_011 = 0xD3)
        assert_encodes_to(
            X64Instr::SubRegReg(Reg::Rbx, Reg::Rdx),
            &[0x48, 0x29, 0xD3],
        );
    }

    #[test]
    fn encodes_imul_rax_rcx_as_48_0f_af_c1() {
        // imul rax, rcx → 48 0F AF C1  (dst=rax=0 in reg, src=rcx=1 in rm,
        // ModR/M = 11_000_001 = 0xC1)
        assert_encodes_to(
            X64Instr::ImulRegReg(Reg::Rax, Reg::Rcx),
            &[0x48, 0x0F, 0xAF, 0xC1],
        );
    }

    #[test]
    fn encodes_push_rax_as_single_50() {
        // push rax → 50 (no REX, default 64-bit operand size in long mode)
        assert_encodes_to(X64Instr::PushReg(Reg::Rax), &[0x50]);
    }

    #[test]
    fn encodes_push_rbp_as_single_55() {
        assert_encodes_to(X64Instr::PushReg(Reg::Rbp), &[0x55]);
    }

    #[test]
    fn encodes_push_r12_with_rex_b() {
        // push r12 → 41 54   (REX.B=1, opcode 50+ (12 & 7) = 54)
        assert_encodes_to(X64Instr::PushReg(Reg::R12), &[0x41, 0x54]);
    }

    #[test]
    fn encodes_pop_rbp_as_single_5d() {
        assert_encodes_to(X64Instr::PopReg(Reg::Rbp), &[0x5D]);
    }

    #[test]
    fn encodes_pop_r15_with_rex_b() {
        // pop r15 → 41 5F
        assert_encodes_to(X64Instr::PopReg(Reg::R15), &[0x41, 0x5F]);
    }

    #[test]
    fn encodes_cmp_rax_rcx_as_48_39_c8() {
        // cmp rax, rcx → 48 39 C8   (rhs=rcx=1 in reg, lhs=rax=0 in rm,
        // ModR/M = 11_001_000 = 0xC8)
        assert_encodes_to(
            X64Instr::CmpRegReg(Reg::Rax, Reg::Rcx),
            &[0x48, 0x39, 0xC8],
        );
    }

    #[test]
    fn encodes_jmp_rel32_zero_as_e9_plus_four_zeros() {
        assert_encodes_to(
            X64Instr::JmpRel32(0),
            &[0xE9, 0x00, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn encodes_jmp_rel32_negative_five_little_endian() {
        // -5 as i32 LE = FB FF FF FF
        assert_encodes_to(
            X64Instr::JmpRel32(-5),
            &[0xE9, 0xFB, 0xFF, 0xFF, 0xFF],
        );
    }

    #[test]
    fn encodes_je_rel32_as_0f_84_disp() {
        assert_encodes_to(
            X64Instr::Je(0x10),
            &[0x0F, 0x84, 0x10, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn encodes_jne_rel32_as_0f_85_disp() {
        assert_encodes_to(
            X64Instr::Jne(0x10),
            &[0x0F, 0x85, 0x10, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn encodes_jl_rel32_as_0f_8c_disp() {
        assert_encodes_to(
            X64Instr::Jl(0x10),
            &[0x0F, 0x8C, 0x10, 0x00, 0x00, 0x00],
        );
    }

    #[test]
    fn encodes_jg_rel32_as_0f_8f_disp() {
        assert_encodes_to(
            X64Instr::Jg(0x10),
            &[0x0F, 0x8F, 0x10, 0x00, 0x00, 0x00],
        );
    }

    // ---- 2. REX prefix correctness for r8-r15 (multiple flavours) -----

    #[test]
    fn rex_prefix_for_mov_r8_to_rax_sets_rex_r() {
        // mov rax, r8 → src=r8 (extended ⇒ REX.R=1), dst=rax (rm, no REX.B)
        //   REX = 0x4C   opcode = 89   ModR/M = 11_000_000 = 0xC0
        assert_encodes_to(
            X64Instr::MovRegReg(Reg::Rax, Reg::R8),
            &[0x4C, 0x89, 0xC0],
        );
    }

    #[test]
    fn rex_prefix_for_mov_r8_to_r9_sets_rex_r_and_rex_b() {
        // mov r9, r8 → src=r8 (REX.R=1), dst=r9 (REX.B=1) ⇒ REX = 0x4D
        //   ModR/M = 11_000_001 = 0xC1   (reg=r8&7=0, rm=r9&7=1)
        assert_encodes_to(
            X64Instr::MovRegReg(Reg::R9, Reg::R8),
            &[0x4D, 0x89, 0xC1],
        );
    }

    #[test]
    fn rex_prefix_for_add_r15_r14_uses_4d_01_f7() {
        // add r15, r14 → REX 4D, opcode 01, ModR/M = 11_110_111 = 0xF7
        //   reg = r14 & 7 = 6, rm = r15 & 7 = 7
        assert_encodes_to(
            X64Instr::AddRegReg(Reg::R15, Reg::R14),
            &[0x4D, 0x01, 0xF7],
        );
    }

    #[test]
    fn rex_prefix_for_imul_r8_rax_sets_only_rex_r() {
        // imul r8, rax → dst=r8 in reg (REX.R=1), src=rax in rm (no REX.B)
        //   REX = 0x4C   opcode = 0F AF   ModR/M = 11_000_000 = 0xC0
        assert_encodes_to(
            X64Instr::ImulRegReg(Reg::R8, Reg::Rax),
            &[0x4C, 0x0F, 0xAF, 0xC0],
        );
    }

    // ---- 3. Intel SDM spot-check tests (at least 5) -------------------

    /// SDM Vol 2B p. 4-35: `MOV RAX, imm64` with W=1 and opcode B8.
    #[test]
    fn sdm_spotcheck_mov_rax_imm64_value_42() {
        // mov rax, 42 → 48 B8 2A 00 00 00 00 00 00 00
        assert_encodes_to(
            X64Instr::MovRegImm64(Reg::Rax, 42),
            &[0x48, 0xB8, 0x2A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        );
    }

    /// SDM Vol 2A p. 3-29: `ADD r/m64, r64` opcode 01 /r.
    /// `add rdx, rsi` is the example in many textbook references.
    #[test]
    fn sdm_spotcheck_add_rdx_rsi_yields_48_01_f2() {
        // src=rsi=6 in reg, dst=rdx=2 in rm ⇒ ModR/M = 11_110_010 = 0xF2
        assert_encodes_to(
            X64Instr::AddRegReg(Reg::Rdx, Reg::Rsi),
            &[0x48, 0x01, 0xF2],
        );
    }

    /// SDM Vol 2B p. 4-279: `PUSH r64` opcode 50+rd, no REX.W, no operand-
    /// size override. `push rsp` uses opcode 54.
    #[test]
    fn sdm_spotcheck_push_rsp_is_single_54() {
        assert_encodes_to(X64Instr::PushReg(Reg::Rsp), &[0x54]);
    }

    /// SDM Vol 2A p. 3-587: `Jcc rel32` two-byte opcode `0F 8x`.
    /// The displacement is signed and relative to the next instruction.
    #[test]
    fn sdm_spotcheck_je_negative_displacement_sign_extends() {
        // je -2  →  0F 84 FE FF FF FF  (a tight infinite loop back to self
        // when this instruction is itself 6 bytes long)
        assert_encodes_to(
            X64Instr::Je(-2),
            &[0x0F, 0x84, 0xFE, 0xFF, 0xFF, 0xFF],
        );
    }

    /// SDM Vol 2B p. 4-685: `SYSCALL` opcode 0F 05; SDM Vol 2B p. 4-441:
    /// `RET` opcode C3. Combined as a trivial leaf function.
    #[test]
    fn sdm_spotcheck_syscall_then_ret_concatenates_correctly() {
        let bytes = encode(&[X64Instr::Syscall, X64Instr::Ret]);
        assert_eq!(bytes, vec![0x0F, 0x05, 0xC3]);
    }

    // ---- 4. Hello world fixture (exact bytes) -------------------------

    #[test]
    fn hello_world_program_encodes_to_known_fixture() {
        let prog = emit_hello_world_program();
        let bytes = encode(&prog);

        // 4 × (10 byte mov imm64) + 2 byte syscall + 2 × (10 byte mov imm64) + 2 byte syscall
        //   = 40 + 2 + 20 + 2 = 64 bytes.
        let expected: [u8; 64] = [
            // mov rax, 1
            0x48, 0xB8, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // mov rdi, 1
            0x48, 0xBF, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // mov rsi, 0x0000000000400000  (placeholder data address)
            0x48, 0xBE, 0x00, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00,
            // mov rdx, 18    (length of "Hello from Orison\n")
            0x48, 0xBA, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // syscall
            0x0F, 0x05,
            // mov rax, 60   (exit)
            0x48, 0xB8, 0x3C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // mov rdi, 0
            0x48, 0xBF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // syscall
            0x0F, 0x05,
        ];
        assert_eq!(bytes.as_slice(), &expected[..]);
        assert_eq!(bytes.len(), 64);
    }

    #[test]
    fn hello_world_string_literal_is_eighteen_bytes() {
        // The instruction stream embeds the string length 18 in rdx; if the
        // canonical greeting ever changes this test must change in lockstep.
        let s = "Hello from Orison\n";
        assert_eq!(s.len(), 18);
    }

    // ---- 5. Determinism ---------------------------------------------

    #[test]
    fn encode_is_deterministic_across_many_runs() {
        let prog = emit_hello_world_program();
        let first = encode(&prog);
        for _ in 0..16 {
            let again = encode(&prog);
            assert_eq!(again, first, "encode produced different bytes on re-run");
        }
    }

    #[test]
    fn encode_with_report_is_deterministic() {
        let prog = vec![
            X64Instr::MovRegImm64(Reg::Rax, 0x1234_5678_9ABC_DEF0u64 as i64),
            X64Instr::AddRegReg(Reg::Rax, Reg::Rcx),
            X64Instr::Ret,
        ];
        let (bytes_a, report_a) = encode_with_report(&prog);
        let (bytes_b, report_b) = encode_with_report(&prog);
        assert_eq!(bytes_a, bytes_b);
        assert_eq!(report_a, report_b);
    }

    // ---- 6. Round-trip: instruction_count / byte_count agree ---------

    #[test]
    fn round_trip_report_matches_program_shape() {
        let prog = vec![
            X64Instr::Nop,
            X64Instr::PushReg(Reg::Rbp),                  // 1 byte
            X64Instr::MovRegReg(Reg::Rbp, Reg::Rsp),      // 3 bytes
            X64Instr::SubRegReg(Reg::Rsp, Reg::Rax),      // 3 bytes
            X64Instr::PopReg(Reg::Rbp),                   // 1 byte
            X64Instr::Ret,                                // 1 byte
        ];
        let (bytes, report) = encode_with_report(&prog);

        // Expected byte count: 1 + 1 + 3 + 3 + 1 + 1 = 10.
        assert_eq!(bytes.len(), 10);
        assert_eq!(report.byte_count, 10);
        assert_eq!(report.instruction_count, prog.len());
        assert_eq!(report.instruction_count, 6);
        assert_eq!(report.schema, X64_REPORT_SCHEMA);
        // FNV-1a hash format is `fnv1a:<16 hex chars>`.
        assert!(report.hash.starts_with("fnv1a:"));
        assert_eq!(report.hash.len(), "fnv1a:".len() + 16);
        assert!(report
            .hash
            .trim_start_matches("fnv1a:")
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    // ---- 7. Hash sanity ---------------------------------------------

    #[test]
    fn hash_of_empty_program_is_offset_basis() {
        let (bytes, report) = encode_with_report(&[]);
        assert!(bytes.is_empty());
        assert_eq!(report.byte_count, 0);
        assert_eq!(report.instruction_count, 0);
        // FNV-1a of zero bytes is just the offset basis.
        assert_eq!(report.hash, format!("fnv1a:{:016x}", FNV1A_OFFSET_BASIS));
    }

    #[test]
    fn hash_changes_when_a_single_byte_changes() {
        let (_, report_a) = encode_with_report(&[X64Instr::MovRegImm64(Reg::Rax, 0)]);
        let (_, report_b) = encode_with_report(&[X64Instr::MovRegImm64(Reg::Rax, 1)]);
        assert_ne!(report_a.hash, report_b.hash);
        assert_eq!(report_a.byte_count, report_b.byte_count);
    }

    // ---- 8. Reg helpers ---------------------------------------------

    #[test]
    fn reg_index_low3_and_extended_classification() {
        for (reg, idx) in [
            (Reg::Rax, 0u8),
            (Reg::Rcx, 1),
            (Reg::Rdx, 2),
            (Reg::Rbx, 3),
            (Reg::Rsp, 4),
            (Reg::Rbp, 5),
            (Reg::Rsi, 6),
            (Reg::Rdi, 7),
            (Reg::R8, 8),
            (Reg::R9, 9),
            (Reg::R10, 10),
            (Reg::R11, 11),
            (Reg::R12, 12),
            (Reg::R13, 13),
            (Reg::R14, 14),
            (Reg::R15, 15),
        ] {
            assert_eq!(reg.index(), idx, "wrong index for {:?}", reg);
            assert_eq!(reg.low3(), idx & 0b111, "wrong low3 for {:?}", reg);
            assert_eq!(
                reg.is_extended(),
                idx >= 8,
                "wrong is_extended for {:?}",
                reg
            );
        }
    }

    // ---- 9. REX / ModR/M primitives ---------------------------------

    #[test]
    fn rex_byte_layout_matches_intel_spec() {
        assert_eq!(rex(0, 0, 0, 0), 0x40);
        assert_eq!(rex(1, 0, 0, 0), 0x48);
        assert_eq!(rex(1, 0, 0, 1), 0x49);
        assert_eq!(rex(1, 1, 0, 0), 0x4C);
        assert_eq!(rex(1, 1, 0, 1), 0x4D);
    }

    #[test]
    fn modrm_byte_layout_packs_fields_correctly() {
        // mod=11, reg=001, rm=000  →  11_001_000 = 0xC8
        assert_eq!(modrm(0b11, 0b001, 0b000), 0xC8);
        // mod=11, reg=110, rm=111  →  11_110_111 = 0xF7
        assert_eq!(modrm(0b11, 0b110, 0b111), 0xF7);
    }
}
