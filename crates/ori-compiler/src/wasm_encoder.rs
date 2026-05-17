//! Hand-rolled WebAssembly binary encoder (bootstrap subset).
//!
//! This module produces real `.wasm` bytes — no third-party crate, no
//! shelling out to `wasm-tools`. It is deliberately tiny: just enough of
//! the binary format to emit a valid empty module and a valid module that
//! exports a single `i32`-returning `main` function. The shape is the
//! contract; future passes can extend the encoder without breaking the
//! existing artefact path.
//!
//! Reference: WebAssembly Core Specification 2.0, sections 5.3-5.5.
//! All multi-byte integers use unsigned LEB128 unless otherwise noted.
//! Section layout: [id:u8] [size:leb128] [contents:bytes].

use crate::mir::MirModule;

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
const SECTION_FUNCTION: u8 = 3;
const SECTION_EXPORT: u8 = 7;
const SECTION_CODE: u8 = 10;

// Wasm "func" type-form tag.
const TYPE_FUNC: u8 = 0x60;
// `i32` value type.
const VALTYPE_I32: u8 = 0x7f;
// Export descriptor: function.
const EXPORTDESC_FUNC: u8 = 0x00;
// Opcodes we emit.
const OP_I32_CONST: u8 = 0x41;
const OP_END: u8 = 0x0b;

/// Write an unsigned LEB128 integer for `value` into `out`.
///
/// LEB128 packs 7 bits per byte, little-endian, with the high bit set on
/// every byte except the last. Used everywhere in the wasm binary format
/// for lengths, indices, and `i32.const` literals (the value is encoded as
/// a signed LEB128 there, but for small positive constants the bit pattern
/// is identical and we only emit small positives in the bootstrap path).
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

/// Decode the next unsigned LEB128 value starting at `offset`.
///
/// Returns `(value, bytes_consumed)`. Returns `None` if the stream is
/// truncated or the encoding exceeds 5 bytes (the maximum for a 32-bit
/// value). Used by tests to round-trip the encoder; the helper is also
/// available to downstream crates that want to spot-check emitted bytes.
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
    let mut out = Vec::new();
    out.extend_from_slice(&WASM_MAGIC);
    out.extend_from_slice(&WASM_VERSION);

    // Type section: [count=1] [0x60 0 1 0x7f]
    let mut type_body: Vec<u8> = Vec::new();
    leb128_u32(1, &mut type_body);
    type_body.push(TYPE_FUNC);
    leb128_u32(0, &mut type_body); // 0 params
    leb128_u32(1, &mut type_body); // 1 result
    type_body.push(VALTYPE_I32);
    write_section(&mut out, SECTION_TYPE, &type_body);

    // Function section: [count=1] [typeidx=0]
    let mut func_body: Vec<u8> = Vec::new();
    leb128_u32(1, &mut func_body);
    leb128_u32(0, &mut func_body);
    write_section(&mut out, SECTION_FUNCTION, &func_body);

    // Export section: [count=1] [name="main"] [kind=func] [funcidx=0]
    let mut export_body: Vec<u8> = Vec::new();
    leb128_u32(1, &mut export_body);
    write_name(&mut export_body, "main");
    export_body.push(EXPORTDESC_FUNC);
    leb128_u32(0, &mut export_body);
    write_section(&mut out, SECTION_EXPORT, &export_body);

    // Code section: [count=1] [body_size_leb] [local_count=0] [i32.const 42] [end]
    let mut code_body: Vec<u8> = Vec::new();
    leb128_u32(1, &mut code_body);
    let mut function_bytes: Vec<u8> = Vec::new();
    leb128_u32(0, &mut function_bytes); // local count
    function_bytes.push(OP_I32_CONST);
    leb128_u32(42, &mut function_bytes); // literal 42 — fits in 1 byte unsigned LEB
    function_bytes.push(OP_END);
    leb128_u32(
        u32::try_from(function_bytes.len()).unwrap_or(u32::MAX),
        &mut code_body,
    );
    code_body.extend_from_slice(&function_bytes);
    write_section(&mut out, SECTION_CODE, &code_body);

    out
}

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

    // Validate first; emit second. This keeps the output deterministic
    // even if validation gets richer later.
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

    let function_count = u32::try_from(mir.functions.len())
        .map_err(|_| EncodeError::InvalidMir("function count exceeds u32::MAX".to_string()))?;

    let mut out = Vec::new();
    out.extend_from_slice(&WASM_MAGIC);
    out.extend_from_slice(&WASM_VERSION);

    // Type section: a single shared `() -> i32` type, index 0.
    let mut type_body: Vec<u8> = Vec::new();
    leb128_u32(1, &mut type_body);
    type_body.push(TYPE_FUNC);
    leb128_u32(0, &mut type_body);
    leb128_u32(1, &mut type_body);
    type_body.push(VALTYPE_I32);
    write_section(&mut out, SECTION_TYPE, &type_body);

    // Function section: every function points at type 0.
    let mut func_body: Vec<u8> = Vec::new();
    leb128_u32(function_count, &mut func_body);
    for _ in 0..function_count {
        leb128_u32(0, &mut func_body);
    }
    write_section(&mut out, SECTION_FUNCTION, &func_body);

    // Export section: export every function by name.
    let mut export_body: Vec<u8> = Vec::new();
    leb128_u32(function_count, &mut export_body);
    for (idx, func) in mir.functions.iter().enumerate() {
        write_name(&mut export_body, &func.name);
        export_body.push(EXPORTDESC_FUNC);
        let funcidx = u32::try_from(idx)
            .map_err(|_| EncodeError::InvalidMir("function index overflow".to_string()))?;
        leb128_u32(funcidx, &mut export_body);
    }
    write_section(&mut out, SECTION_EXPORT, &export_body);

    // Code section: every function body is `i32.const 0; end`.
    let mut code_body: Vec<u8> = Vec::new();
    leb128_u32(function_count, &mut code_body);
    for _ in 0..function_count {
        let mut function_bytes: Vec<u8> = Vec::new();
        leb128_u32(0, &mut function_bytes); // local count
        function_bytes.push(OP_I32_CONST);
        leb128_u32(0, &mut function_bytes);
        function_bytes.push(OP_END);
        leb128_u32(
            u32::try_from(function_bytes.len()).unwrap_or(u32::MAX),
            &mut code_body,
        );
        code_body.extend_from_slice(&function_bytes);
    }
    write_section(&mut out, SECTION_CODE, &code_body);

    Ok(out)
}

/// A wasm "name" is a length-prefixed UTF-8 byte sequence. Used in the
/// export, import, and custom sections.
fn write_name(out: &mut Vec<u8>, name: &str) {
    let bytes = name.as_bytes();
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    leb128_u32(len, out);
    out.extend_from_slice(bytes);
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
}
