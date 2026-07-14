#![no_std]

//! Allocation-free compiler for the strict NovaRust language subset.
//! The caller owns the source, compiler workspace and output buffers.

#[cfg(test)]
extern crate std;

mod ast;
mod codegen;
mod diagnostic;
mod elf;
mod ir;
mod lexer;
mod parser;
mod types;

pub use ast::{Ast, BinaryOp, Expr, ExprId, Function, Stmt, StmtId, UnaryOp};
pub use codegen::{CodeImage, Relocation, RelocationKind, Symbol};
pub use diagnostic::{CompileError, Diagnostic, ErrorCode, Span};
pub use elf::{ElfImage, ELF_BASE, ELF_ENTRY};
pub use ir::{Block, BlockId, Inst, IrFunction, IrProgram, ValueId};
pub use lexer::{Lexer, Token, TokenKind};
pub use types::Type;

use codegen::Emitter;
use parser::Parser;

pub const MAX_TOKENS: usize = 512;

/// Reusable, fixed-capacity compiler storage.
pub struct Workspace<'s> {
    pub ast: Ast<'s>,
    pub ir: IrProgram,
}

impl<'s> Workspace<'s> {
    pub const fn new() -> Self {
        Self {
            ast: Ast::new(),
            ir: IrProgram::new(),
        }
    }
}

impl Default for Workspace<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Compile NovaRust source into an executable ELF64 image in `output`.
pub fn compile<'s>(
    source: &'s str,
    workspace: &mut Workspace<'s>,
    output: &mut [u8],
) -> Result<ElfImage, CompileError> {
    workspace.ast.clear();
    workspace.ir.clear();
    Parser::new(source, &mut workspace.ast).parse_program()?;
    types::check(&mut workspace.ast)?;
    ir::lower(&workspace.ast, &mut workspace.ir)?;
    ir::optimize(&mut workspace.ir);
    let mut code = [0u8; codegen::MAX_CODE];
    let image = Emitter::new(&workspace.ast, &workspace.ir, &mut code).emit()?;
    elf::write_elf(output, &image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_end_to_end_to_inspectable_elf() {
        let source = "fn add(a: i64, b: i64) -> i64 { return a + b; } fn main() -> i64 { let mut x: i64 = 40; x = add(x, 2); if x == 42 { return x; } return 1; }";
        let mut ws = Workspace::new();
        let mut out = [0u8; 8192];
        let image = compile(source, &mut ws, &mut out).unwrap();
        assert_eq!(&out[0..4], b"\x7fELF");
        assert_eq!(out[4], 2); // ELFCLASS64
        assert_eq!(u16::from_le_bytes([out[18], out[19]]), 62); // EM_X86_64
        let entry = u64::from_le_bytes(out[24..32].try_into().unwrap());
        assert_eq!(entry, ELF_ENTRY);
        assert!(image.len > 120);
        assert!(ws.ast.functions().any(|f| f.name == "main"));
        assert!(ws.ir.function_count() >= 2);
    }

    #[test]
    fn diagnostics_are_ukrainian() {
        let mut ws = Workspace::new();
        let mut out = [0u8; 1024];
        let err = compile("fn main() -> i64 { return true; }", &mut ws, &mut out).unwrap_err();
        assert!(err.diagnostic().message.contains("тип"));
    }

    #[test]
    fn outer_locals_survive_nested_control_flow() {
        let source = "fn main() -> i64 { let mut x: i64 = 40; if true { let y: i64 = 2; x = x + y; } while false { x = 0; } return x; }";
        let mut ws = Workspace::new();
        let mut out = [0u8; 8192];
        assert!(compile(source, &mut ws, &mut out).is_ok());
    }

    #[test]
    fn emits_nova_abi_intrinsics() {
        let source = "fn main() -> i64 { nova_print_byte(42); nova_yield(); return 0; }";
        let mut ws = Workspace::new();
        let mut out = [0u8; 8192];
        let image = compile(source, &mut ws, &mut out).unwrap();
        assert!(out[image.code_offset..image.code_offset + image.code_len]
            .windows(2)
            .any(|bytes| bytes == [0xcd, 0x80]));
    }
}
