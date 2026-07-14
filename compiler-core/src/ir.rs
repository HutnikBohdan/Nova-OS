use crate::{Ast, CompileError, ErrorCode, Span};

pub const MAX_IR_FUNCTIONS: usize = 32;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ValueId(pub u16);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockId(pub u16);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Inst {
    Return(ValueId),
    Constant(ValueId, i64),
    Nop,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IrFunction {
    pub ast_index: u8,
    pub blocks: u8,
}
pub struct IrProgram {
    functions: [IrFunction; MAX_IR_FUNCTIONS],
    len: usize,
}
impl IrProgram {
    pub const fn new() -> Self {
        Self {
            functions: [IrFunction {
                ast_index: 0,
                blocks: 0,
            }; MAX_IR_FUNCTIONS],
            len: 0,
        }
    }
    pub fn clear(&mut self) {
        self.len = 0;
    }
    pub fn function_count(&self) -> usize {
        self.len
    }
}
impl Default for IrProgram {
    fn default() -> Self {
        Self::new()
    }
}
pub fn lower(ast: &Ast<'_>, output: &mut IrProgram) -> Result<(), CompileError> {
    output.clear();
    for (index, _) in ast.functions().enumerate() {
        if output.len == MAX_IR_FUNCTIONS {
            return Err(CompileError::new(
                ErrorCode::Capacity,
                Span::default(),
                "забагато IR-функцій",
            ));
        }
        output.functions[output.len] = IrFunction {
            ast_index: index as u8,
            blocks: 1,
        };
        output.len += 1;
    }
    Ok(())
}
pub fn optimize(_program: &mut IrProgram) {}
