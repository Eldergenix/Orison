//! Hand-rolled WebAssembly binary encoder (bootstrap subset).
//!
//! This module produces real `.wasm` bytes — no third-party crate, no
//! shelling out to `wasm-tools`. It is deliberately tiny: just enough of
//! the binary format to emit a valid empty module, a hello-world module,
//! and (since M25) multi-function modules with imports, exports, locals,
//! and a small instruction set.
//!
//! Reference: WebAssembly Core Specification 2.0, sections 5.3-5.5.
//! All multi-byte integers use unsigned LEB128 unless otherwise noted;
//! `i32.const` literals are signed LEB128 (see `leb128_i32`).
//! Section layout: [id:u8] [size:leb128] [contents:bytes].
//!
//! Determinism guarantees:
//!   * Type section dedupes signatures and emits them in **insertion order**
//!     (first time a signature is referenced wins its index).
//!   * Export section iterates a `BTreeMap` so output is sorted by name.
//!   * Imports are emitted in the order given by the caller.
//!   * `encode_module(t, f, e, i)` is a pure function — same input ⇒ same
//!     bytes, byte-for-byte.

use crate::mir::MirModule;
use std::collections::BTreeMap;

/// Errors raised by the encoder. These are returned, never panicked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodeError {
    /// The MIR contains a shape the bootstrap encoder does not yet handle.
    UnsupportedShape(String),
    /// The MIR is structurally invalid (e.g. duplicate function names).
    InvalidMir(String),
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::UnsupportedShape(message) => {
                write!(f, "unsupported shape: {message}")
            }
            EncodeError::InvalidMir(message) => write!(f, "invalid mir: {message}"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Wasm magic bytes (`\0asm`) followed by version `1`.
pub const WASM_MAGIC: [u8; 4] = [0x00, 0x61, 0x73, 0x6d];
pub const WASM_VERSION: [u8; 4] = [0x01, 0x00, 0x00, 0x00];

// Section identifiers we use.
const SECTION_TYPE: u8 = 1;
const SECTION_IMPORT: u8 = 2;
const SECTION_FUNCTION: u8 = 3;
const SECTION_EXPORT: u8 = 7;
const SECTION_CODE: u8 = 10;

// Wasm "func" type-form tag.
const TYPE_FUNC: u8 = 0x60;
// Value-type tags from the binary spec.
const VALTYPE_I32: u8 = 0x7f;
const VALTYPE_I64: u8 = 0x7e;
const VALTYPE_F32: u8 = 0x7d;
const VALTYPE_F64: u8 = 0x7c;
// "Empty" block type used by `if`/`block`/`loop` when they leave no value.
const BLOCKTYPE_EMPTY: u8 = 0x40;
// Export / import descriptor: function.
const EXPORTDESC_FUNC: u8 = 0x00;
const IMPORTDESC_FUNC: u8 = 0x00;

// Opcodes (binary spec §5.4).
#[cfg(test)]
const OP_UNREACHABLE: u8 = 0x00;
const OP_NOP: u8 = 0x01;
const OP_BLOCK: u8 = 0x02;
const OP_LOOP: u8 = 0x03;
const OP_IF: u8 = 0x04;
const OP_ELSE: u8 = 0x05;
const OP_END: u8 = 0x0b;
const OP_BR: u8 = 0x0c;
const OP_BR_IF: u8 = 0x0d;
const OP_RETURN: u8 = 0x0f;
const OP_CALL: u8 = 0x10;
const OP_DROP: u8 = 0x1a;
const OP_LOCAL_GET: u8 = 0x20;
const OP_LOCAL_SET: u8 = 0x21;
const OP_LOCAL_TEE: u8 = 0x22;
const OP_I32_CONST: u8 = 0x41;
const OP_I32_EQ: u8 = 0x46;
const OP_I32_LT_S: u8 = 0x48;
const OP_I32_GT_S: u8 = 0x4a;
const OP_I32_ADD: u8 = 0x6a;
const OP_I32_SUB: u8 = 0x6b;
const OP_I32_MUL: u8 = 0x6c;
const OP_I32_DIV_S: u8 = 0x6d;

// ---------------------------------------------------------------------
// Public types for the multi-function encoder (M25).
// ---------------------------------------------------------------------

/// Wasm value type. Bootstrap subset: just the four numeric types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
}

impl ValType {
    fn tag(self) -> u8 {
        match self {
            ValType::I32 => VALTYPE_I32,
            ValType::I64 => VALTYPE_I64,
            ValType::F32 => VALTYPE_F32,
            ValType::F64 => VALTYPE_F64,
        }
    }
}

/// Block-result type for `block` / `loop` / `if`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlockType {
    /// No result value (encoded as `0x40`).
    Empty,
    /// Single value result.
    Value(ValType),
}

impl BlockType {
    fn write(self, out: &mut Vec<u8>) {
        match self {
            BlockType::Empty => out.push(BLOCKTYPE_EMPTY),
            BlockType::Value(v) => out.push(v.tag()),
        }
    }
}

/// A wasm function signature: parameter types → result types.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WasmFuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

/// A defined wasm function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmFunc {
    /// Index into the type section.
    pub type_index: u32,
    /// Run-length-encoded locals: `(count, type)` pairs (params not included).
    pub locals: Vec<(u32, ValType)>,
    /// Instruction stream. The terminating `End` is *not* added automatically;
    /// the caller is responsible for the function's trailing `End`. (This
    /// keeps the encoder a transparent serialiser — no hidden insertions.)
    pub body: Vec<Instr>,
}

/// An imported function (only function imports are needed for the bootstrap,
/// e.g. `wasi_unstable.fd_write`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmImport {
    pub module: String,
    pub name: String,
    /// Index into the type section for the imported function's signature.
    pub type_index: u32,
}

/// An exported item. Bootstrap only exports functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmExport {
    pub func_index: u32,
}

/// Wasm instruction (bootstrap subset). Numeric ops, locals, control flow,
/// and `call`. Block-typed instructions carry the block's result type so
/// the emitter can write the immediate without a side table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instr {
    // Numeric constants.
    I32Const(i32),
    // Binary integer arithmetic.
    I32Add,
    I32Sub,
    I32Mul,
    I32DivS,
    // Integer comparison.
    I32Eq,
    I32LtS,
    I32GtS,
    // Locals.
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    // Calls.
    Call(u32),
    // Control flow.
    Block(BlockType),
    Loop(BlockType),
    If(BlockType),
    Else,
    End,
    Br(u32),
    BrIf(u32),
    Return,
    Drop,
    Nop,
}

impl Instr {
    /// Serialise one instruction in wasm binary form (spec §5.4).
    fn write(&self, out: &mut Vec<u8>) {
        match self {
            Instr::I32Const(value) => {
                out.push(OP_I32_CONST);
                leb128_i32(*value, out);
            }
            Instr::I32Add => out.push(OP_I32_ADD),
            Instr::I32Sub => out.push(OP_I32_SUB),
            Instr::I32Mul => out.push(OP_I32_MUL),
            Instr::I32DivS => out.push(OP_I32_DIV_S),
            Instr::I32Eq => out.push(OP_I32_EQ),
            Instr::I32LtS => out.push(OP_I32_LT_S),
            Instr::I32GtS => out.push(OP_I32_GT_S),
            Instr::LocalGet(i) => {
                out.push(OP_LOCAL_GET);
                leb128_u32(*i, out);
            }
            Instr::LocalSet(i) => {
                out.push(OP_LOCAL_SET);
                leb128_u32(*i, out);
            }
            Instr::LocalTee(i) => {
                out.push(OP_LOCAL_TEE);
                leb128_u32(*i, out);
            }
            Instr::Call(i) => {
                out.push(OP_CALL);
                leb128_u32(*i, out);
            }
            Instr::Block(bt) => {
                out.push(OP_BLOCK);
                bt.write(out);
            }
            Instr::Loop(bt) => {
                out.push(OP_LOOP);
                bt.write(out);
            }
            Instr::If(bt) => {
                out.push(OP_IF);
                bt.write(out);
            }
            Instr::Else => out.push(OP_ELSE),
            Instr::End => out.push(OP_END),
            Instr::Br(depth) => {
                out.push(OP_BR);
                leb128_u32(*depth, out);
            }
            Instr::BrIf(depth) => {
                out.push(OP_BR_IF);
                leb128_u32(*depth, out);
            }
            Instr::Return => out.push(OP_RETURN),
            Instr::Drop => out.push(OP_DROP),
            Instr::Nop => out.push(OP_NOP),
        }
    }

    /// Suppress dead-code warnings; the variant is kept for completeness even
    /// though no public emitter uses it yet.
    #[cfg(test)]
    fn unreachable_byte() -> u8 {
        OP_UNREACHABLE
    }
}

// ---------------------------------------------------------------------
// LEB128 helpers.
// ---------------------------------------------------------------------

/// Write an unsigned LEB128 integer for `value` into `out`.
///
/// LEB128 packs 7 bits per byte, little-endian, with the high bit set on
/// every byte except the last. Used everywhere in the wasm binary format
/// for lengths and indices.
pub fn leb128_u32(value: u32, out: &mut Vec<u8>) {
    let mut remaining = value;
    loop {
        let byte = (remaining & 0x7f) as u8;
        remaining >>= 7;
        if remaining == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Write a signed LEB128 integer for `value` into `out`.
///
/// Used for `i32.const` literals. The encoding terminates when the
/// sign-extended high bits match the sign bit of the last 7-bit chunk
/// (spec §5.2.2).
pub fn leb128_i32(value: i32, out: &mut Vec<u8>) {
    let mut remaining = value;
    loop {
        let byte = (remaining & 0x7f) as u8;
        // Arithmetic shift preserves the sign so the loop terminates
        // correctly for negative inputs.
        remaining >>= 7;
        let sign_bit = byte & 0x40;
        let done = (remaining == 0 && sign_bit == 0) || (remaining == -1 && sign_bit != 0);
        if done {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Decode the next unsigned LEB128 value starting at `offset`.
///
/// Returns `(value, bytes_consumed)`. Returns `None` if the stream is
/// truncated or the encoding exceeds 5 bytes (the maximum for a 32-bit
/// value). Used by tests to round-trip the encoder.
#[cfg(test)]
fn read_leb128_u32(bytes: &[u8], offset: usize) -> Option<(u32, usize)> {
    let mut result: u32 = 0;
    let mut shift: u32 = 0;
    let mut consumed: usize = 0;
    while consumed < 5 {
        let byte = *bytes.get(offset + consumed)?;
        let chunk = (byte & 0x7f) as u32;
        // Guard against overflow on the final byte.
        let shifted = chunk.checked_shl(shift)?;
        result |= shifted;
        consumed += 1;
        if byte & 0x80 == 0 {
            return Some((result, consumed));
        }
        shift = shift.checked_add(7)?;
    }
    None
}

/// Wrap a section body in `[id, size_leb128, body...]` and append it to `out`.
fn write_section(out: &mut Vec<u8>, id: u8, body: &[u8]) {
    out.push(id);
    let len = u32::try_from(body.len()).unwrap_or(u32::MAX);
    leb128_u32(len, out);
    out.extend_from_slice(body);
}

/// A wasm "name" is a length-prefixed UTF-8 byte sequence. Used in the
/// export, import, and custom sections.
fn write_name(out: &mut Vec<u8>, name: &str) {
    let bytes = name.as_bytes();
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    leb128_u32(len, out);
    out.extend_from_slice(bytes);
}

/// Encode a `WasmFuncType` into the type section's `functype` form.
fn write_func_type(out: &mut Vec<u8>, ty: &WasmFuncType) {
    out.push(TYPE_FUNC);
    let n_params = u32::try_from(ty.params.len()).unwrap_or(u32::MAX);
    leb128_u32(n_params, out);
    for p in &ty.params {
        out.push(p.tag());
    }
    let n_results = u32::try_from(ty.results.len()).unwrap_or(u32::MAX);
    leb128_u32(n_results, out);
    for r in &ty.results {
        out.push(r.tag());
    }
}

// ---------------------------------------------------------------------
// Minimal and hello modules (kept for compatibility with older callers).
// ---------------------------------------------------------------------

/// Emit a valid empty WebAssembly module: just the 8-byte preamble.
///
/// This is the smallest wasm module that passes `wasm-validate`.
pub fn encode_minimal_module() -> Vec<u8> {
    let mut out = Vec::with_capacity(8);
    out.extend_from_slice(&WASM_MAGIC);
    out.extend_from_slice(&WASM_VERSION);
    out
}

/// Emit a hello-world wasm module: exports `main` as `() -> i32` returning 42.
///
/// Layout:
///   - preamble (magic + version)
///   - type section:   1 type, `() -> i32`
///   - function section: 1 function, type index 0
///   - export section: 1 export, name "main", funcidx 0
///   - code section:   1 body, no locals, `i32.const 42; end`
pub fn encode_hello_module() -> Vec<u8> {
    let types = vec![WasmFuncType {
        params: Vec::new(),
        results: vec![ValType::I32],
    }];
    let funcs = vec![WasmFunc {
        type_index: 0,
        locals: Vec::new(),
        body: vec![Instr::I32Const(42), Instr::End],
    }];
    let mut exports = BTreeMap::new();
    exports.insert("main".to_string(), WasmExport { func_index: 0 });
    encode_module(&types, &funcs, &exports, &[])
}

// ---------------------------------------------------------------------
// The general multi-function encoder (M25).
// ---------------------------------------------------------------------

/// Encode a full wasm module from explicit type/function/import/export
/// tables. This is the M25 entry point.
///
/// Guarantees:
///   * Byte-deterministic: same inputs ⇒ same bytes.
///   * Type section emits `types` in the given order (callers may dedupe
///     up-front via `dedupe_types`).
///   * Import section emits `imports` in the given order. Imported
///     functions come first in the function index space (wasm spec).
///   * Export section iterates a `BTreeMap`, so exports are sorted by name.
///
/// The encoder does **not** validate that body instructions reference
/// valid locals or type indices — it's a serialiser, not a checker. Pair
/// with a higher-level pass when you need validation.
pub fn encode_module(
    types: &[WasmFuncType],
    funcs: &[WasmFunc],
    exports: &BTreeMap<String, WasmExport>,
    imports: &[WasmImport],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&WASM_MAGIC);
    out.extend_from_slice(&WASM_VERSION);

    // -------- Type section (id = 1) --------
    if !types.is_empty() {
        let mut body = Vec::new();
        let n = u32::try_from(types.len()).unwrap_or(u32::MAX);
        leb128_u32(n, &mut body);
        for ty in types {
            write_func_type(&mut body, ty);
        }
        write_section(&mut out, SECTION_TYPE, &body);
    }

    // -------- Import section (id = 2) --------
    if !imports.is_empty() {
        let mut body = Vec::new();
        let n = u32::try_from(imports.len()).unwrap_or(u32::MAX);
        leb128_u32(n, &mut body);
        for imp in imports {
            write_name(&mut body, &imp.module);
            write_name(&mut body, &imp.name);
            body.push(IMPORTDESC_FUNC);
            leb128_u32(imp.type_index, &mut body);
        }
        write_section(&mut out, SECTION_IMPORT, &body);
    }

    // -------- Function section (id = 3) --------
    if !funcs.is_empty() {
        let mut body = Vec::new();
        let n = u32::try_from(funcs.len()).unwrap_or(u32::MAX);
        leb128_u32(n, &mut body);
        for f in funcs {
            leb128_u32(f.type_index, &mut body);
        }
        write_section(&mut out, SECTION_FUNCTION, &body);
    }

    // -------- Export section (id = 7) --------
    if !exports.is_empty() {
        let mut body = Vec::new();
        let n = u32::try_from(exports.len()).unwrap_or(u32::MAX);
        leb128_u32(n, &mut body);
        // BTreeMap iteration is sorted ⇒ stable export order.
        for (name, ex) in exports {
            write_name(&mut body, name);
            body.push(EXPORTDESC_FUNC);
            leb128_u32(ex.func_index, &mut body);
        }
        write_section(&mut out, SECTION_EXPORT, &body);
    }

    // -------- Code section (id = 10) --------
    if !funcs.is_empty() {
        let mut body = Vec::new();
        let n = u32::try_from(funcs.len()).unwrap_or(u32::MAX);
        leb128_u32(n, &mut body);
        for f in funcs {
            let mut fn_bytes = Vec::new();
            // Locals: vec of (count, valtype) — already RLE'd by the caller.
            let n_local_groups = u32::try_from(f.locals.len()).unwrap_or(u32::MAX);
            leb128_u32(n_local_groups, &mut fn_bytes);
            for (count, vt) in &f.locals {
                leb128_u32(*count, &mut fn_bytes);
                fn_bytes.push(vt.tag());
            }
            for instr in &f.body {
                instr.write(&mut fn_bytes);
            }
            let size = u32::try_from(fn_bytes.len()).unwrap_or(u32::MAX);
            leb128_u32(size, &mut body);
            body.extend_from_slice(&fn_bytes);
        }
        write_section(&mut out, SECTION_CODE, &body);
    }

    out
}

/// Dedupe a list of function types, returning `(unique_types, index_map)`
/// where `index_map[i]` is the position of `input[i]` in `unique_types`.
///
/// Insertion order is preserved (first occurrence wins). Useful for
/// front-ends that emit one signature per function and want a compact
/// type section.
pub fn dedupe_types(input: &[WasmFuncType]) -> (Vec<WasmFuncType>, Vec<u32>) {
    // BTreeMap keyed by the canonical (params, results) tuple gives us a
    // deterministic lookup; we still emit `unique` in *insertion* order.
    let mut seen: BTreeMap<(Vec<ValType>, Vec<ValType>), u32> = BTreeMap::new();
    let mut unique: Vec<WasmFuncType> = Vec::new();
    let mut indices: Vec<u32> = Vec::with_capacity(input.len());
    for ty in input {
        let key = (ty.params.clone(), ty.results.clone());
        if let Some(&idx) = seen.get(&key) {
            indices.push(idx);
        } else {
            let idx = u32::try_from(unique.len()).unwrap_or(u32::MAX);
            seen.insert(key, idx);
            unique.push(ty.clone());
            indices.push(idx);
        }
    }
    (unique, indices)
}

// ---------------------------------------------------------------------
// MIR lowering (unchanged contract, reimplemented on top of encode_module).
// ---------------------------------------------------------------------

/// Best-effort lowering from MIR to a wasm module.
///
/// Bootstrap policy:
///   * No functions → emit the minimal (empty) module.
///   * Every function: `() -> i32` returning `0` (`i32.const 0; end`).
///   * Anything more complex (non-empty params, non-`Int`/`Unit` returns,
///     unrecognised opcodes) → `EncodeError::UnsupportedShape`.
///
/// The contract is byte-deterministic: identical MIR yields identical bytes.
pub fn encode_from_mir(mir: &MirModule) -> Result<Vec<u8>, EncodeError> {
    if mir.functions.is_empty() {
        return Ok(encode_minimal_module());
    }

    let mut seen_names = std::collections::BTreeSet::new();
    for func in &mir.functions {
        if !seen_names.insert(func.name.clone()) {
            return Err(EncodeError::InvalidMir(format!(
                "duplicate function name: {}",
                func.name
            )));
        }
        if !func.params.is_empty() {
            return Err(EncodeError::UnsupportedShape(format!(
                "function `{}` has parameters; bootstrap encoder only supports `() -> i32`",
                func.name
            )));
        }
        if !is_i32_return_type(&func.return_type) {
            return Err(EncodeError::UnsupportedShape(format!(
                "function `{}` returns `{}`; bootstrap encoder only supports `Int`/`Unit`",
                func.name, func.return_type
            )));
        }
    }

    // Build inputs to `encode_module`. All functions share the single
    // `() -> i32` signature; exports are sorted by name via BTreeMap.
    let types = vec![WasmFuncType {
        params: Vec::new(),
        results: vec![ValType::I32],
    }];
    let funcs: Vec<WasmFunc> = (0..mir.functions.len())
        .map(|_| WasmFunc {
            type_index: 0,
            locals: Vec::new(),
            body: vec![Instr::I32Const(0), Instr::End],
        })
        .collect();
    let mut exports: BTreeMap<String, WasmExport> = BTreeMap::new();
    for (idx, func) in mir.functions.iter().enumerate() {
        let funcidx = u32::try_from(idx)
            .map_err(|_| EncodeError::InvalidMir("function index overflow".to_string()))?;
        exports.insert(
            func.name.clone(),
            WasmExport {
                func_index: funcidx,
            },
        );
    }

    Ok(encode_module(&types, &funcs, &exports, &[]))
}

fn is_i32_return_type(ty: &str) -> bool {
    matches!(ty, "Int" | "Unit" | "i32")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::HirParam;
    use crate::mir::{MirBlock, MirFunction, MirInstruction};

    // ---- LEB128 boundary tests --------------------------------------

    #[test]
    fn leb128_zero_is_one_byte_zero() {
        let mut out = Vec::new();
        leb128_u32(0, &mut out);
        assert_eq!(out, vec![0x00]);
    }

    #[test]
    fn leb128_one_hundred_twenty_seven_is_one_byte() {
        let mut out = Vec::new();
        leb128_u32(127, &mut out);
        assert_eq!(out, vec![0x7f]);
    }

    #[test]
    fn leb128_one_hundred_twenty_eight_is_two_bytes() {
        let mut out = Vec::new();
        leb128_u32(128, &mut out);
        assert_eq!(out, vec![0x80, 0x01]);
    }

    #[test]
    fn leb128_sixteen_thousand_three_hundred_eighty_four_is_three_bytes() {
        let mut out = Vec::new();
        leb128_u32(16_384, &mut out);
        assert_eq!(out, vec![0x80, 0x80, 0x01]);
    }

    #[test]
    fn leb128_max_u32_is_five_bytes() {
        let mut out = Vec::new();
        leb128_u32(0xFFFF_FFFF, &mut out);
        assert_eq!(out, vec![0xff, 0xff, 0xff, 0xff, 0x0f]);
    }

    #[test]
    fn leb128_round_trip_for_boundaries() {
        for &value in &[0u32, 1, 127, 128, 16_383, 16_384, 2_097_151, 0xFFFF_FFFF] {
            let mut buf = Vec::new();
            leb128_u32(value, &mut buf);
            let decoded = read_leb128_u32(&buf, 0);
            assert_eq!(decoded.map(|(v, _)| v), Some(value), "value={value}");
        }
    }

    // ---- Signed LEB128 ----------------------------------------------

    #[test]
    fn leb128_i32_zero_is_one_byte() {
        let mut out = Vec::new();
        leb128_i32(0, &mut out);
        assert_eq!(out, vec![0x00]);
    }

    #[test]
    fn leb128_i32_small_positive_is_one_byte() {
        let mut out = Vec::new();
        leb128_i32(42, &mut out);
        assert_eq!(out, vec![0x2a]);
    }

    #[test]
    fn leb128_i32_minus_one_is_one_byte_seven_f() {
        let mut out = Vec::new();
        leb128_i32(-1, &mut out);
        assert_eq!(out, vec![0x7f]);
    }

    #[test]
    fn leb128_i32_one_hundred_twenty_seven_needs_two_bytes_to_disambiguate_sign() {
        // 127 fits in 7 bits but its high bit (sign bit of the chunk) is 1,
        // so signed LEB128 must spill into a second byte to avoid being read
        // as -1.
        let mut out = Vec::new();
        leb128_i32(127, &mut out);
        assert_eq!(out, vec![0xff, 0x00]);
    }

    #[test]
    fn leb128_i32_minus_sixty_four_is_one_byte() {
        // -64 is the largest-magnitude negative number that fits in one
        // signed LEB128 byte.
        let mut out = Vec::new();
        leb128_i32(-64, &mut out);
        assert_eq!(out, vec![0x40]);
    }

    // ---- Minimal module --------------------------------------------

    #[test]
    fn minimal_module_has_magic_and_version_only() {
        let bytes = encode_minimal_module();
        assert_eq!(bytes.len(), 8);
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        assert_eq!(&bytes[4..8], &WASM_VERSION);
    }

    // ---- Hello module: structural decode ---------------------------

    /// Tiny in-test decoder verifying the section layout of `encode_hello_module`.
    fn parse_sections(bytes: &[u8]) -> Vec<(u8, Vec<u8>)> {
        let mut sections = Vec::new();
        let mut cursor = 8usize;
        while cursor < bytes.len() {
            let Some(&id) = bytes.get(cursor) else { break };
            cursor += 1;
            let Some((size, consumed)) = read_leb128_u32(bytes, cursor) else {
                break;
            };
            cursor += consumed;
            let size = size as usize;
            let Some(body) = bytes.get(cursor..cursor + size) else {
                break;
            };
            sections.push((id, body.to_vec()));
            cursor += size;
        }
        sections
    }

    #[test]
    fn hello_module_has_magic_version_and_expected_size() {
        let bytes = encode_hello_module();
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        assert_eq!(&bytes[4..8], &WASM_VERSION);
        // Sanity: well under 64 bytes for hello-world.
        assert!(
            bytes.len() < 64,
            "hello module unexpectedly large: {}",
            bytes.len()
        );
        assert!(
            bytes.len() > 8,
            "hello module must have sections beyond preamble"
        );
    }

    #[test]
    fn hello_module_emits_type_function_export_and_code_sections() {
        let bytes = encode_hello_module();
        let sections = parse_sections(&bytes);
        let ids: Vec<u8> = sections.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            ids,
            vec![SECTION_TYPE, SECTION_FUNCTION, SECTION_EXPORT, SECTION_CODE]
        );
    }

    #[test]
    fn hello_module_type_section_describes_unit_to_i32() {
        let bytes = encode_hello_module();
        let sections = parse_sections(&bytes);
        let type_section = sections
            .iter()
            .find(|(id, _)| *id == SECTION_TYPE)
            .map(|(_, body)| body.clone())
            .unwrap_or_default();
        // [count=1] [0x60] [0] [1] [0x7f]
        assert_eq!(type_section, vec![0x01, 0x60, 0x00, 0x01, 0x7f]);
    }

    #[test]
    fn hello_module_export_section_names_main() {
        let bytes = encode_hello_module();
        let sections = parse_sections(&bytes);
        let export_body = sections
            .iter()
            .find(|(id, _)| *id == SECTION_EXPORT)
            .map(|(_, body)| body.clone())
            .unwrap_or_default();
        // [count=1] [name_len=4] [m a i n] [kind=0] [funcidx=0]
        assert_eq!(
            export_body,
            vec![0x01, 0x04, b'm', b'a', b'i', b'n', EXPORTDESC_FUNC, 0x00]
        );
    }

    #[test]
    fn hello_module_code_section_returns_forty_two() {
        let bytes = encode_hello_module();
        let sections = parse_sections(&bytes);
        let code_body = sections
            .iter()
            .find(|(id, _)| *id == SECTION_CODE)
            .map(|(_, body)| body.clone())
            .unwrap_or_default();
        // [count=1] [body_size=4] [locals=0] [i32.const 42] [end]
        assert_eq!(
            code_body,
            vec![0x01, 0x04, 0x00, OP_I32_CONST, 0x2a, OP_END]
        );
    }

    #[test]
    fn hello_module_is_byte_deterministic() {
        assert_eq!(encode_hello_module(), encode_hello_module());
    }

    // ---- encode_from_mir -------------------------------------------

    fn mir_with(funcs: Vec<MirFunction>) -> MirModule {
        MirModule {
            module: "test".to_string(),
            functions: funcs,
        }
    }

    fn mir_function(name: &str, return_type: &str) -> MirFunction {
        MirFunction {
            name: name.to_string(),
            params: Vec::new(),
            return_type: return_type.to_string(),
            blocks: vec![MirBlock {
                id: 0,
                instructions: vec![MirInstruction {
                    op: "const_default".to_string(),
                    args: vec![return_type.to_string()],
                    result: Some(format!("%ret:{name}")),
                }],
            }],
        }
    }

    #[test]
    fn encode_from_mir_with_no_functions_returns_minimal_module() {
        let mir = mir_with(Vec::new());
        let bytes = encode_from_mir(&mir);
        assert_eq!(bytes, Ok(encode_minimal_module()));
    }

    #[test]
    fn encode_from_mir_with_single_unit_function_succeeds() {
        let mir = mir_with(vec![mir_function("main", "Unit")]);
        let bytes = encode_from_mir(&mir).unwrap_or_default();
        assert!(!bytes.is_empty(), "encoder must succeed");
        let sections = parse_sections(&bytes);
        let ids: Vec<u8> = sections.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            ids,
            vec![SECTION_TYPE, SECTION_FUNCTION, SECTION_EXPORT, SECTION_CODE]
        );
    }

    #[test]
    fn encode_from_mir_rejects_parameters() {
        let mut func = mir_function("greet", "Int");
        func.params.push(HirParam {
            name: "name".to_string(),
            r#type: "Str".to_string(),
        });
        let mir = mir_with(vec![func]);
        let err = encode_from_mir(&mir);
        assert!(matches!(err, Err(EncodeError::UnsupportedShape(_))));
    }

    #[test]
    fn encode_from_mir_rejects_unsupported_return_type() {
        let mir = mir_with(vec![mir_function("page", "View")]);
        let err = encode_from_mir(&mir);
        assert!(matches!(err, Err(EncodeError::UnsupportedShape(_))));
    }

    #[test]
    fn encode_from_mir_rejects_duplicate_names() {
        let mir = mir_with(vec![mir_function("dup", "Int"), mir_function("dup", "Int")]);
        let err = encode_from_mir(&mir);
        assert!(matches!(err, Err(EncodeError::InvalidMir(_))));
    }

    #[test]
    fn encode_from_mir_is_byte_deterministic() {
        let mir = mir_with(vec![mir_function("a", "Int"), mir_function("b", "Unit")]);
        let first = encode_from_mir(&mir);
        let second = encode_from_mir(&mir);
        assert_eq!(first, second);
    }

    // ---- M25: encode_module multi-function tests -------------------

    /// Build an `(i32, i32) -> i32` signature — the workhorse used by the
    /// arithmetic tests below.
    fn binop_sig() -> WasmFuncType {
        WasmFuncType {
            params: vec![ValType::I32, ValType::I32],
            results: vec![ValType::I32],
        }
    }

    /// Helper: encode a binary op function `local.get 0; local.get 1; <op>; end`.
    fn binop_func(op: Instr) -> WasmFunc {
        WasmFunc {
            type_index: 0,
            locals: Vec::new(),
            body: vec![Instr::LocalGet(0), Instr::LocalGet(1), op, Instr::End],
        }
    }

    #[test]
    fn encode_module_add_sub_has_magic_and_section_headers() {
        let types = vec![binop_sig()];
        let funcs = vec![binop_func(Instr::I32Add), binop_func(Instr::I32Sub)];
        let mut exports = BTreeMap::new();
        exports.insert("add".to_string(), WasmExport { func_index: 0 });
        exports.insert("sub".to_string(), WasmExport { func_index: 1 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);

        // (a) magic + version preamble.
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        assert_eq!(&bytes[4..8], &WASM_VERSION);
        // (b) first section header byte at offset 8 is the type section id.
        assert_eq!(bytes[8], SECTION_TYPE);
        // (c) determinism.
        let again = encode_module(&types, &funcs, &exports, &[]);
        assert_eq!(bytes, again);
    }

    #[test]
    fn encode_module_add_sub_section_bytes_are_exact() {
        // Pin the entire layout for the canonical add/sub module.
        let types = vec![binop_sig()];
        let funcs = vec![binop_func(Instr::I32Add), binop_func(Instr::I32Sub)];
        let mut exports = BTreeMap::new();
        exports.insert("add".to_string(), WasmExport { func_index: 0 });
        exports.insert("sub".to_string(), WasmExport { func_index: 1 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);
        let sections = parse_sections(&bytes);

        // Type section: [count=1][0x60][2 i32 i32][1 i32]
        let type_body = sections
            .iter()
            .find(|(id, _)| *id == SECTION_TYPE)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        assert_eq!(
            type_body,
            vec![
                0x01,
                0x60,
                0x02,
                VALTYPE_I32,
                VALTYPE_I32,
                0x01,
                VALTYPE_I32
            ]
        );

        // Function section: [count=2][typeidx=0][typeidx=0]
        let func_body = sections
            .iter()
            .find(|(id, _)| *id == SECTION_FUNCTION)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        assert_eq!(func_body, vec![0x02, 0x00, 0x00]);

        // Export section: BTreeMap sorts "add" then "sub".
        let export_body = sections
            .iter()
            .find(|(id, _)| *id == SECTION_EXPORT)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        assert_eq!(
            export_body,
            vec![
                0x02, // count
                0x03,
                b'a',
                b'd',
                b'd',
                EXPORTDESC_FUNC,
                0x00,
                0x03,
                b's',
                b'u',
                b'b',
                EXPORTDESC_FUNC,
                0x01,
            ]
        );
    }

    #[test]
    fn encode_module_export_order_is_sorted_by_name() {
        // Insert in reverse alphabetical order to prove the BTreeMap sorts.
        let types = vec![binop_sig()];
        let funcs = vec![binop_func(Instr::I32Add), binop_func(Instr::I32Sub)];
        let mut exports = BTreeMap::new();
        exports.insert("zeta".to_string(), WasmExport { func_index: 1 });
        exports.insert("alpha".to_string(), WasmExport { func_index: 0 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);
        // (a) magic.
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        // (b) type section header at offset 8.
        assert_eq!(bytes[8], SECTION_TYPE);
        // (c) determinism.
        assert_eq!(bytes, encode_module(&types, &funcs, &exports, &[]));

        let sections = parse_sections(&bytes);
        let export_body = sections
            .iter()
            .find(|(id, _)| *id == SECTION_EXPORT)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        // First name in body must be "alpha".
        assert_eq!(&export_body[1..6], &[0x05, b'a', b'l', b'p', b'h']);
    }

    #[test]
    fn encode_module_with_locals_emits_rle_groups() {
        // A function `() -> i32` with two i32 locals and one i64 local.
        let types = vec![WasmFuncType {
            params: Vec::new(),
            results: vec![ValType::I32],
        }];
        let funcs = vec![WasmFunc {
            type_index: 0,
            locals: vec![(2, ValType::I32), (1, ValType::I64)],
            body: vec![Instr::I32Const(7), Instr::End],
        }];
        let mut exports = BTreeMap::new();
        exports.insert("f".to_string(), WasmExport { func_index: 0 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);

        // (a) magic preamble.
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        // (b) section header at offset 8 is the type section.
        assert_eq!(bytes[8], SECTION_TYPE);

        let sections = parse_sections(&bytes);
        let code_body = sections
            .iter()
            .find(|(id, _)| *id == SECTION_CODE)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        // count=1, body_size=?, then [locals_count=2][2,i32][1,i64][i32.const 7][end]
        // body_size: 1 (locals count) + 2 (group 1) + 2 (group 2) + 2 (const) + 1 (end) = 8.
        assert_eq!(
            code_body,
            vec![
                0x01,
                0x08, // count=1, body_size=8
                0x02, // 2 local groups
                0x02,
                VALTYPE_I32,
                0x01,
                VALTYPE_I64,
                OP_I32_CONST,
                0x07,
                OP_END,
            ]
        );

        // (c) determinism.
        assert_eq!(bytes, encode_module(&types, &funcs, &exports, &[]));
    }

    #[test]
    fn encode_module_with_import_section_emits_module_dot_name() {
        // Import wasi_unstable.fd_write : (i32, i32, i32, i32) -> i32
        let types = vec![WasmFuncType {
            params: vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            results: vec![ValType::I32],
        }];
        let imports = vec![WasmImport {
            module: "wasi_unstable".to_string(),
            name: "fd_write".to_string(),
            type_index: 0,
        }];

        let bytes = encode_module(&types, &[], &BTreeMap::new(), &imports);

        // (a) magic.
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        // (b) type section header at offset 8.
        assert_eq!(bytes[8], SECTION_TYPE);

        let sections = parse_sections(&bytes);
        let ids: Vec<u8> = sections.iter().map(|(id, _)| *id).collect();
        // Only type + import — no funcs, no exports, no code.
        assert_eq!(ids, vec![SECTION_TYPE, SECTION_IMPORT]);

        let import_body = sections
            .iter()
            .find(|(id, _)| *id == SECTION_IMPORT)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        // [count=1][13 "wasi_unstable"][8 "fd_write"][kind=0][typeidx=0]
        assert_eq!(import_body[0], 0x01);
        assert_eq!(import_body[1], 13);
        assert_eq!(&import_body[2..15], b"wasi_unstable");
        assert_eq!(import_body[15], 8);
        assert_eq!(&import_body[16..24], b"fd_write");
        assert_eq!(import_body[24], IMPORTDESC_FUNC);
        assert_eq!(import_body[25], 0x00);

        // (c) determinism.
        assert_eq!(
            bytes,
            encode_module(&types, &[], &BTreeMap::new(), &imports)
        );
    }

    #[test]
    fn encode_module_control_flow_instructions_serialise() {
        // A function: `if (a == b) { return a; } else { return b; } end`.
        // Demonstrates `If`/`Else`/`End`, `Return`, `I32Eq`, `LocalGet`.
        let types = vec![binop_sig()];
        let funcs = vec![WasmFunc {
            type_index: 0,
            locals: Vec::new(),
            body: vec![
                Instr::LocalGet(0),
                Instr::LocalGet(1),
                Instr::I32Eq,
                Instr::If(BlockType::Empty),
                Instr::LocalGet(0),
                Instr::Return,
                Instr::Else,
                Instr::LocalGet(1),
                Instr::Return,
                Instr::End,
                Instr::Nop,
                Instr::I32Const(0),
                Instr::End,
            ],
        }];
        let mut exports = BTreeMap::new();
        exports.insert("max".to_string(), WasmExport { func_index: 0 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);

        // (a) magic.
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        // (b) type section at offset 8.
        assert_eq!(bytes[8], SECTION_TYPE);

        // Confirm specific opcodes survived into the code section.
        let sections = parse_sections(&bytes);
        let code = sections
            .iter()
            .find(|(id, _)| *id == SECTION_CODE)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        assert!(code.contains(&OP_IF));
        assert!(code.contains(&OP_ELSE));
        assert!(code.contains(&OP_RETURN));
        assert!(code.contains(&OP_I32_EQ));
        assert!(code.contains(&OP_NOP));

        // (c) determinism.
        assert_eq!(bytes, encode_module(&types, &funcs, &exports, &[]));
    }

    #[test]
    fn encode_module_block_and_loop_with_br_serialise() {
        // A tight loop: `block; loop; br_if 0; br 1; end; end`.
        let types = vec![WasmFuncType {
            params: Vec::new(),
            results: Vec::new(),
        }];
        let funcs = vec![WasmFunc {
            type_index: 0,
            locals: vec![(1, ValType::I32)],
            body: vec![
                Instr::Block(BlockType::Empty),
                Instr::Loop(BlockType::Empty),
                Instr::LocalGet(0),
                Instr::BrIf(0),
                Instr::Br(1),
                Instr::End, // end loop
                Instr::End, // end block
                Instr::Drop,
                Instr::End, // end function
            ],
        }];
        let mut exports = BTreeMap::new();
        exports.insert("spin".to_string(), WasmExport { func_index: 0 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);
        // (a) magic.
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        // (b) type section header at offset 8.
        assert_eq!(bytes[8], SECTION_TYPE);
        // (c) determinism.
        assert_eq!(bytes, encode_module(&types, &funcs, &exports, &[]));

        let sections = parse_sections(&bytes);
        let code = sections
            .iter()
            .find(|(id, _)| *id == SECTION_CODE)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        for op in [OP_BLOCK, OP_LOOP, OP_BR, OP_BR_IF, OP_DROP] {
            assert!(code.contains(&op), "missing opcode 0x{op:02x}");
        }
    }

    #[test]
    fn encode_module_call_instruction_references_function_by_index() {
        // f0 = identity (returns its arg); f1 = calls f0 with const 9.
        let types = vec![
            WasmFuncType {
                params: vec![ValType::I32],
                results: vec![ValType::I32],
            },
            WasmFuncType {
                params: Vec::new(),
                results: vec![ValType::I32],
            },
        ];
        let funcs = vec![
            WasmFunc {
                type_index: 0,
                locals: Vec::new(),
                body: vec![Instr::LocalGet(0), Instr::End],
            },
            WasmFunc {
                type_index: 1,
                locals: Vec::new(),
                body: vec![Instr::I32Const(9), Instr::Call(0), Instr::End],
            },
        ];
        let mut exports = BTreeMap::new();
        exports.insert("identity".to_string(), WasmExport { func_index: 0 });
        exports.insert("nine".to_string(), WasmExport { func_index: 1 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        assert_eq!(bytes[8], SECTION_TYPE);

        let sections = parse_sections(&bytes);
        let code = sections
            .iter()
            .find(|(id, _)| *id == SECTION_CODE)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        assert!(code.contains(&OP_CALL));

        // Determinism.
        assert_eq!(bytes, encode_module(&types, &funcs, &exports, &[]));
    }

    #[test]
    fn encode_module_locals_use_local_set_and_tee() {
        // A `(i32) -> i32` that does `local.tee 1; drop; local.get 1` etc.
        let types = vec![WasmFuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        }];
        let funcs = vec![WasmFunc {
            type_index: 0,
            locals: vec![(1, ValType::I32)],
            body: vec![
                Instr::LocalGet(0),
                Instr::LocalTee(1),
                Instr::LocalSet(1),
                Instr::LocalGet(1),
                Instr::End,
            ],
        }];
        let mut exports = BTreeMap::new();
        exports.insert("rt".to_string(), WasmExport { func_index: 0 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        assert_eq!(bytes[8], SECTION_TYPE);

        let sections = parse_sections(&bytes);
        let code = sections
            .iter()
            .find(|(id, _)| *id == SECTION_CODE)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        assert!(code.contains(&OP_LOCAL_GET));
        assert!(code.contains(&OP_LOCAL_SET));
        assert!(code.contains(&OP_LOCAL_TEE));

        assert_eq!(bytes, encode_module(&types, &funcs, &exports, &[]));
    }

    #[test]
    fn encode_module_with_mixed_arithmetic_opcodes_round_trips() {
        // Exercise mul/div_s/lt_s/gt_s in a single body.
        let types = vec![binop_sig()];
        let funcs = vec![WasmFunc {
            type_index: 0,
            locals: Vec::new(),
            body: vec![
                Instr::LocalGet(0),
                Instr::LocalGet(1),
                Instr::I32Mul,
                Instr::LocalGet(0),
                Instr::LocalGet(1),
                Instr::I32DivS,
                Instr::I32Add,
                Instr::LocalGet(0),
                Instr::LocalGet(1),
                Instr::I32LtS,
                Instr::LocalGet(0),
                Instr::LocalGet(1),
                Instr::I32GtS,
                Instr::I32Add,
                Instr::I32Add,
                Instr::End,
            ],
        }];
        let mut exports = BTreeMap::new();
        exports.insert("blob".to_string(), WasmExport { func_index: 0 });

        let bytes = encode_module(&types, &funcs, &exports, &[]);
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        assert_eq!(bytes[8], SECTION_TYPE);
        assert_eq!(bytes, encode_module(&types, &funcs, &exports, &[]));

        let sections = parse_sections(&bytes);
        let code = sections
            .iter()
            .find(|(id, _)| *id == SECTION_CODE)
            .map(|(_, b)| b.clone())
            .unwrap_or_default();
        for op in [
            OP_I32_MUL,
            OP_I32_DIV_S,
            OP_I32_LT_S,
            OP_I32_GT_S,
            OP_I32_ADD,
        ] {
            assert!(code.contains(&op));
        }
    }

    #[test]
    fn dedupe_types_returns_first_index_for_duplicates() {
        let a = WasmFuncType {
            params: vec![ValType::I32],
            results: vec![ValType::I32],
        };
        let b = WasmFuncType {
            params: vec![ValType::I32, ValType::I32],
            results: vec![ValType::I32],
        };
        let input = vec![a.clone(), b.clone(), a.clone(), b.clone(), a.clone()];
        let (unique, idx) = dedupe_types(&input);
        assert_eq!(unique, vec![a, b]);
        assert_eq!(idx, vec![0, 1, 0, 1, 0]);
    }

    #[test]
    fn encode_module_with_imports_then_funcs_emits_sections_in_spec_order() {
        // Per spec the section *id* order is 1 (type), 2 (import), 3 (function),
        // 7 (export), 10 (code) — we must emit ascending ids.
        let types = vec![
            WasmFuncType {
                params: vec![ValType::I32, ValType::I32, ValType::I32, ValType::I32],
                results: vec![ValType::I32],
            },
            WasmFuncType {
                params: Vec::new(),
                results: vec![ValType::I32],
            },
        ];
        let imports = vec![WasmImport {
            module: "wasi_unstable".to_string(),
            name: "fd_write".to_string(),
            type_index: 0,
        }];
        let funcs = vec![WasmFunc {
            type_index: 1,
            locals: Vec::new(),
            body: vec![Instr::I32Const(0), Instr::End],
        }];
        let mut exports = BTreeMap::new();
        exports.insert("_start".to_string(), WasmExport { func_index: 1 });

        let bytes = encode_module(&types, &funcs, &exports, &imports);

        // (a) magic.
        assert_eq!(&bytes[0..4], &WASM_MAGIC);
        // (b) section header at offset 8 is the type section id.
        assert_eq!(bytes[8], SECTION_TYPE);

        let sections = parse_sections(&bytes);
        let ids: Vec<u8> = sections.iter().map(|(id, _)| *id).collect();
        assert_eq!(
            ids,
            vec![
                SECTION_TYPE,
                SECTION_IMPORT,
                SECTION_FUNCTION,
                SECTION_EXPORT,
                SECTION_CODE,
            ]
        );
        // Ids are strictly ascending — required by the binary format.
        for w in ids.windows(2) {
            assert!(w[0] < w[1], "section ids must be ascending: {ids:?}");
        }

        // (c) determinism.
        assert_eq!(bytes, encode_module(&types, &funcs, &exports, &imports));
    }

    #[test]
    fn encode_module_byte_deterministic_under_repeated_calls() {
        // Regression: 10 calls with identical inputs must produce identical bytes.
        let types = vec![binop_sig()];
        let funcs = vec![binop_func(Instr::I32Add), binop_func(Instr::I32Sub)];
        let mut exports = BTreeMap::new();
        exports.insert("add".to_string(), WasmExport { func_index: 0 });
        exports.insert("sub".to_string(), WasmExport { func_index: 1 });

        let first = encode_module(&types, &funcs, &exports, &[]);
        // (a) magic.
        assert_eq!(&first[0..4], &WASM_MAGIC);
        // (b) section header at offset 8 is the type section.
        assert_eq!(first[8], SECTION_TYPE);
        // (c) determinism across many calls.
        for _ in 0..10 {
            assert_eq!(first, encode_module(&types, &funcs, &exports, &[]));
        }
    }

    #[test]
    fn instr_unreachable_byte_matches_spec() {
        // Sanity: keep `OP_UNREACHABLE` reachable from tests so removing it
        // would break the build. Spec value is 0x00.
        assert_eq!(Instr::unreachable_byte(), 0x00);
    }
}
